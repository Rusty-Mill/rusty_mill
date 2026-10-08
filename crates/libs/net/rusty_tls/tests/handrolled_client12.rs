//! The TLS 1.2 client handshake — stage 4b-iii.
//!
//! # Why a live peer is the test that matters here
//!
//! The previous two stages were checked against recorded OpenSSL bytes. A state
//! machine needs more: a handshake is a mutual computation, and a client that
//! orders the transcript wrongly, signs nothing it should have checked, or
//! encodes an extension slightly off would still agree with a test written by
//! the same hand. rustls has not read this implementation, so a completed
//! handshake against it is evidence about TLS 1.2 rather than about internal
//! consistency. `handrolled_client12_socket_interop.rs` repeats the exercise
//! against OpenSSL over a real socket.
//!
//! # The refusals are the other half
//!
//! A client that completes a handshake with a good server and also with an
//! attacker is worse than none. The second half of this file drives a scripted
//! server (built from this crate's own primitives, used only to make the client
//! *refuse*) and requires each way a flight can be wrong to be refused.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{record_length, ClientError};
use rusty_tls::handrolled::client12::{
    CipherSuite12, ClientConfig12, ClientHandshake12, Connection12, Incoming12,
};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::x509::Certificate;

const SERVER: &str = "tls12.example";

/// Well inside the generated certificates' validity, and fixed so a test never
/// depends on how long the suite takes to run.
fn options() -> PathOptions {
    PathOptions {
        time: 1_800_000_000, // 2027-01-15
        max_path_length: 8,
        max_signature_checks: 64,
        required_eku: None,
    }
}

const NOT_BEFORE: i64 = 1_577_836_800; // 2020-01-01
const NOT_AFTER: i64 = 1_893_456_000; // 2030-01-01

fn dated(params: &mut CertificateParams) {
    params.not_before = OffsetDateTime::from_unix_timestamp(NOT_BEFORE).expect("not_before");
    params.not_after = OffsetDateTime::from_unix_timestamp(NOT_AFTER).expect("not_after");
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.trim().len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s.trim()[i..i + 2], 16).expect("valid hex"))
        .collect()
}

/// The kind of key a server's leaf certificate carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaf {
    P256,
    P384,
    Ed25519,
    Rsa,
}

impl Leaf {
    fn key(self) -> KeyPair {
        match self {
            Self::P256 => KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256),
            Self::P384 => KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384),
            Self::Ed25519 => KeyPair::generate_for(&rcgen::PKCS_ED25519),
            Self::Rsa => {
                // A throwaway 2048-bit RSA key generated for these tests (ring cannot
                // generate RSA keys). It protects nothing.
                let pkcs8 = PrivatePkcs8KeyDer::from(unhex(include_str!("data/rsa2048_pkcs8.hex")));
                KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8, &rcgen::PKCS_RSA_SHA256)
            }
        }
        .expect("leaf key")
    }
}

/// A CA, and a leaf it issued.
struct Pki {
    root_der: Vec<u8>,
    leaf_der: Vec<u8>,
    leaf_pkcs8: Vec<u8>,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

fn pki(leaf: Leaf, name: &str) -> Pki {
    pki_with(leaf, name, |_| {})
}

/// As [`pki`], with a hook to adjust the leaf's parameters (to expire it).
fn pki_with(leaf: Leaf, name: &str, adjust: impl FnOnce(&mut CertificateParams)) -> Pki {
    let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("root params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    root_params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("tls12 test root".to_string()),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");

    let leaf_key = leaf.key();
    let mut leaf_params = CertificateParams::new(vec![name.to_string()]).expect("leaf params");
    dated(&mut leaf_params);
    adjust(&mut leaf_params);
    let leaf_cert = leaf_params
        .signed_by(&leaf_key, &root, &root_key)
        .expect("leaf");

    Pki {
        root_der: root.der().to_vec(),
        leaf_der: leaf_cert.der().to_vec(),
        leaf_pkcs8: leaf_key.serialize_der(),
        chain: vec![
            CertificateDer::from(leaf_cert.der().to_vec()),
            CertificateDer::from(root.der().to_vec()),
        ],
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    }
}

fn anchor(root_der: &[u8]) -> TrustAnchor<'_> {
    let root = Certificate::parse(root_der).expect("the root parses");
    TrustAnchor::from_certificate(&root)
}

// ---------------------------------------------------------------------------
// A real rustls server, restricted to TLS 1.2
// ---------------------------------------------------------------------------

/// A rustls provider restricted to one suite and one group, so each
/// combination is actually negotiated rather than merely offered.
fn restricted_provider(
    suite: Option<rustls::SupportedCipherSuite>,
    group: Option<&'static dyn rustls::crypto::SupportedKxGroup>,
) -> Arc<rustls::crypto::CryptoProvider> {
    let mut provider = rustls::crypto::ring::default_provider();
    if let Some(suite) = suite {
        provider.cipher_suites = vec![suite];
    }
    if let Some(group) = group {
        provider.kx_groups = vec![group];
    }
    Arc::new(provider)
}

fn rustls_server_with(
    pki: &Pki,
    provider: Arc<rustls::crypto::CryptoProvider>,
    versions: &[&'static rustls::SupportedProtocolVersion],
) -> rustls::ServerConnection {
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(pki.chain.clone(), pki.key.clone_key())
        .expect("server config");
    rustls::ServerConnection::new(Arc::new(config)).expect("server connection")
}

fn rustls_server_12(pki: &Pki) -> rustls::ServerConnection {
    rustls_server_with(
        pki,
        Arc::new(rustls::crypto::ring::default_provider()),
        &[&rustls::version::TLS12],
    )
}

/// Split a byte stream into whole records, leaving any partial tail behind.
fn take_records(stream: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(length) = record_length(stream) {
        if stream.len() < length {
            break;
        }
        out.push(stream.drain(..length).collect());
    }
    out
}

/// Feed bytes to a rustls server and collect whatever it wants to send back,
/// tolerating a server that refuses the handshake (its alert is still queued).
fn pump_server(server: &mut rustls::ServerConnection, input: &[u8]) -> (Vec<u8>, Option<String>) {
    let mut refusal = None;
    if !input.is_empty() {
        let mut cursor = std::io::Cursor::new(input);
        while server.read_tls(&mut cursor).unwrap_or(0) > 0 {
            if let Err(err) = server.process_new_packets() {
                refusal = Some(err.to_string());
                break;
            }
        }
    }
    if refusal.is_none() {
        if let Err(err) = server.process_new_packets() {
            refusal = Some(err.to_string());
        }
    }
    let mut out = Vec::new();
    while server.wants_write() {
        server.write_tls(&mut out).expect("write_tls");
    }
    (out, refusal)
}

struct Established {
    connection: Connection12,
    server: rustls::ServerConnection,
}

fn config<'a>(
    name: &'a str,
    anchors: &'a [TrustAnchor<'a>],
    groups: &'a [NamedGroup],
    suites: &'a [CipherSuite12],
) -> ClientConfig12<'a> {
    ClientConfig12 {
        server_name: ServerName::Dns(name),
        anchors,
        path: options(),
        groups,
        cipher_suites: suites,
    }
}

/// Run a handshake to completion against `server`, optionally corrupting the
/// server's records on the way through. `tamper` sees every record, in order,
/// and returns what the client should receive (`None` drops it).
fn handshake_against(
    mut server: rustls::ServerConnection,
    config: &ClientConfig12<'_>,
    mut tamper: impl FnMut(usize, Vec<u8>) -> Option<Vec<u8>>,
) -> Result<Established, ClientError> {
    let (mut client, mut to_server) = ClientHandshake12::start(config)?;

    let mut seen = 0usize;
    for _ in 0..16 {
        let (from_server, _refusal) = pump_server(&mut server, &to_server);
        to_server.clear();

        let mut stream = from_server;
        for record in take_records(&mut stream) {
            seen += 1;
            let Some(record) = tamper(seen - 1, record) else {
                continue;
            };
            to_server.extend_from_slice(&client.read_record(&record)?);
        }

        if client.is_finished() {
            let connection = client.into_connection()?;
            // The server's last flight is already consumed; deliver ours so its
            // state machine finishes too.
            let _ = pump_server(&mut server, &to_server);
            return Ok(Established { connection, server });
        }
        if to_server.is_empty() {
            break;
        }
    }
    Err(ClientError::Failed)
}

fn handshake_12(pki: &Pki) -> Result<Established, ClientError> {
    let anchors = [anchor(&pki.root_der)];
    let config = config(
        SERVER,
        &anchors,
        &[
            NamedGroup::X25519,
            NamedGroup::SecP256R1,
            NamedGroup::SecP384R1,
        ],
        CipherSuite12::SUPPORTED,
    );
    handshake_against(rustls_server_12(pki), &config, |_, record| Some(record))
}

/// Send application data client to server and read it back on the rustls side.
///
/// Fed one record at a time with a drain in between: rustls holds at most 64 KiB
/// of unread plaintext, so handing it a long stream whole would stop it
/// reading, which is a property of this helper and not of the client.
fn client_to_server(established: &mut Established, data: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut stream = established.connection.write(data).expect("write");
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    for record in take_records(&mut stream) {
        let (_, refusal) = pump_server(&mut established.server, &record);
        assert_eq!(refusal, None, "rustls refused our application data");
        loop {
            match established.server.reader().read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
    }
    got
}

/// Send application data server to client and return what the client read.
///
/// Written in chunks and flushed between them for the mirror reason of
/// [`client_to_server`]: rustls buffers at most 64 KiB of unsent plaintext.
fn server_to_client(established: &mut Established, data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut got = Vec::new();
    for chunk in data.chunks(16 * 1024) {
        established
            .server
            .writer()
            .write_all(chunk)
            .expect("server write");
        let (bytes, _) = pump_server(&mut established.server, &[]);
        let mut stream = bytes;
        for record in take_records(&mut stream) {
            match established.connection.read(&record).expect("client reads") {
                Incoming12::Application(plain) => got.extend_from_slice(&plain),
                other => panic!("expected application data, got {other:?}"),
            }
        }
    }
    got
}

// ---------------------------------------------------------------------------
// Interop — the test that carries this file
// ---------------------------------------------------------------------------

/// A complete handshake against a real rustls TLS 1.2 server, then data both
/// ways, then an orderly close.
#[test]
fn a_full_tls12_handshake_against_rustls_completes_and_carries_data() {
    let pki = pki(Leaf::P256, SERVER);
    let mut est = handshake_12(&pki).expect("handshake completes");

    assert_eq!(est.connection.peer_certificates()[0], pki.leaf_der);
    assert!(est.connection.suite().parts().is_some());
    assert!(
        !est.server.is_handshaking(),
        "rustls considers the handshake done"
    );

    assert_eq!(
        client_to_server(&mut est, b"hello from the hand-rolled client"),
        b"hello from the hand-rolled client"
    );
    assert_eq!(
        server_to_client(&mut est, b"hello from rustls"),
        b"hello from rustls"
    );

    // A fragment larger than one record is split, and arrives whole.
    let big: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(client_to_server(&mut est, &big), big);
    assert_eq!(server_to_client(&mut est, &big), big);
}

/// Every suite, against every curve it can use, with the key type that suite
/// authenticates with: 6 suites x 3 groups x the matching leaf kinds.
#[test]
fn every_suite_and_group_completes_against_rustls() {
    use rustls::crypto::ring::cipher_suite as rs;
    use rustls::crypto::ring::kx_group;

    let suites: [(CipherSuite12, rustls::SupportedCipherSuite, &[Leaf]); 6] = [
        (
            CipherSuite12::ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            rs::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            &[Leaf::P256, Leaf::P384, Leaf::Ed25519],
        ),
        (
            CipherSuite12::ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            rs::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            &[Leaf::P256, Leaf::P384],
        ),
        (
            CipherSuite12::ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            rs::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            &[Leaf::Rsa],
        ),
        (
            CipherSuite12::ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            rs::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            &[Leaf::Rsa],
        ),
        (
            CipherSuite12::ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
            rs::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
            &[Leaf::P256, Leaf::Ed25519],
        ),
        (
            CipherSuite12::ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
            rs::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
            &[Leaf::Rsa],
        ),
    ];
    let groups: [(NamedGroup, &'static dyn rustls::crypto::SupportedKxGroup); 3] = [
        (NamedGroup::X25519, kx_group::X25519),
        (NamedGroup::SecP256R1, kx_group::SECP256R1),
        (NamedGroup::SecP384R1, kx_group::SECP384R1),
    ];

    let mut completed = 0;
    for (ours, theirs, leaves) in suites {
        for leaf in leaves {
            let pki = pki(*leaf, SERVER);
            let anchors = [anchor(&pki.root_der)];
            for (our_group, their_group) in groups {
                let provider = restricted_provider(Some(theirs), Some(their_group));
                let server = rustls_server_with(&pki, provider, &[&rustls::version::TLS12]);
                let suites = [ours];
                let our_groups = [our_group];
                let cfg = config(SERVER, &anchors, &our_groups, &suites);
                let mut est = handshake_against(server, &cfg, |_, r| Some(r))
                    .unwrap_or_else(|e| panic!("{ours:?} / {leaf:?} / {our_group:?}: {e}"));
                assert_eq!(est.connection.suite(), ours);
                assert_eq!(
                    client_to_server(&mut est, b"ping"),
                    b"ping",
                    "{ours:?} {leaf:?}"
                );
                assert_eq!(
                    server_to_client(&mut est, b"pong"),
                    b"pong",
                    "{ours:?} {leaf:?}"
                );
                completed += 1;
            }
        }
    }
    assert_eq!(completed, (3 + 2 + 1 + 1 + 2 + 1) * 3);
}

#[test]
fn close_notify_works_in_both_directions() {
    let pki = pki(Leaf::P256, SERVER);
    let mut est = handshake_12(&pki).expect("handshake");

    // The server closes: the client sees an orderly close, not an error.
    est.server.send_close_notify();
    let (bytes, _) = pump_server(&mut est.server, &[]);
    let mut stream = bytes;
    let records = take_records(&mut stream);
    assert_eq!(records.len(), 1);
    assert_eq!(
        est.connection.read(&records[0]).expect("reads"),
        Incoming12::Closed
    );
    assert!(
        est.connection.write(b"after close").is_err(),
        "no writes after the peer closed"
    );

    // The client closes: rustls reads it as a clean end of stream.
    let mut est = handshake_12(&pki).expect("handshake");
    let alert = est.connection.close().expect("close_notify");
    let (_, refusal) = pump_server(&mut est.server, &alert);
    assert_eq!(refusal, None);
    let mut buf = [0u8; 8];
    use std::io::Read;
    assert_eq!(est.server.reader().read(&mut buf).expect("clean eof"), 0);
}

/// rustls restricted to TLS 1.3 refuses a client that offers only TLS 1.2, and
/// says why: the alert is surfaced, not collapsed into an unexpected record.
#[test]
fn a_tls13_only_server_refuses_with_a_protocol_version_alert() {
    let pki = pki(Leaf::P256, SERVER);
    let anchors = [anchor(&pki.root_der)];
    let cfg = config(
        SERVER,
        &anchors,
        &[NamedGroup::X25519],
        CipherSuite12::SUPPORTED,
    );
    let server = rustls_server_with(
        &pki,
        Arc::new(rustls::crypto::ring::default_provider()),
        &[&rustls::version::TLS13],
    );
    match handshake_against(server, &cfg, |_, r| Some(r)) {
        Err(ClientError::PeerAlert(alert)) => {
            assert_eq!(alert.description.0, 70, "protocol_version")
        }
        Err(other) => panic!("expected the peer's alert, got {other}"),
        Ok(_) => panic!("a TLS 1.3-only server cannot complete a TLS 1.2 handshake"),
    }
}

/// With a required client certificate, rustls refuses this client; with an
/// optional one it completes. This client answers a request with an empty
/// Certificate and no more.
#[test]
fn a_client_certificate_request_is_answered_with_an_empty_certificate() {
    let pki = pki(Leaf::P256, SERVER);
    let anchors = [anchor(&pki.root_der)];
    let cfg = config(
        SERVER,
        &anchors,
        &[NamedGroup::X25519],
        CipherSuite12::SUPPORTED,
    );

    let mut roots = rustls::RootCertStore::empty();
    roots.add(pki.chain[1].clone()).expect("root");
    let build = |optional: bool| {
        let builder = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots.clone()));
        let verifier = if optional {
            builder.allow_unauthenticated()
        } else {
            builder
        }
        .build()
        .expect("verifier");
        let config =
            rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
                .with_client_cert_verifier(verifier)
                .with_single_cert(pki.chain.clone(), pki.key.clone_key())
                .expect("config");
        rustls::ServerConnection::new(Arc::new(config)).expect("server")
    };

    let ok = handshake_against(build(true), &cfg, |_, r| Some(r));
    assert!(ok.is_ok(), "an optional client certificate: {:?}", ok.err());

    let refused = handshake_against(build(false), &cfg, |_, r| Some(r));
    assert!(
        refused.is_err(),
        "a required client certificate cannot be satisfied with none"
    );
}

// ---------------------------------------------------------------------------
// What the client sends
// ---------------------------------------------------------------------------

mod hello {
    use super::*;
    use rusty_tls::handrolled::handshake::{extension, messages, ClientHello, HandshakeType};

    fn client_hello(name: ServerName<'_>) -> Vec<u8> {
        let anchors: [TrustAnchor<'_>; 0] = [];
        let cfg = ClientConfig12 {
            server_name: name,
            anchors: &anchors,
            path: options(),
            groups: &[
                NamedGroup::X25519,
                NamedGroup::SecP256R1,
                NamedGroup::SecP384R1,
            ],
            cipher_suites: CipherSuite12::SUPPORTED,
        };
        let (_, record) = ClientHandshake12::start(&cfg).expect("starts");
        assert_eq!(record[0], 22, "a handshake record");
        record
    }

    fn parse(record: &[u8], then: impl FnOnce(&ClientHello<'_>)) {
        let msgs = messages(&record[5..]).expect("one message");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].typ, HandshakeType::ClientHello);
        then(&ClientHello::parse(msgs[0].body).expect("parses"));
    }

    #[test]
    fn the_hello_offers_what_it_should_and_nothing_it_should_not() {
        parse(&client_hello(ServerName::Dns(SERVER)), |hello| {
            assert_eq!(hello.random.len(), 32);
            assert!(
                hello.session_id.is_empty(),
                "no session id: resumption is not offered"
            );

            // Exactly the six AEAD ECDHE suites. No CBC, RC4, 3DES, static RSA,
            // DHE, SHA-1 or the TLS 1.3 suites, and no SCSV.
            let expected: Vec<u16> = CipherSuite12::SUPPORTED.iter().map(|s| s.0).collect();
            assert_eq!(hello.cipher_suites, expected);

            let types: Vec<u16> = hello.extensions.iter().map(|e| e.typ).collect();
            for wanted in [
                extension::SERVER_NAME,
                extension::SUPPORTED_GROUPS,
                extension::EC_POINT_FORMATS,
                extension::SIGNATURE_ALGORITHMS,
                extension::EXTENDED_MASTER_SECRET,
                extension::RENEGOTIATION_INFO,
            ] {
                assert!(
                    types.contains(&wanted),
                    "extension {wanted} is missing: {types:?}"
                );
            }
            // Nothing that would invite TLS 1.3, resumption or a feature this
            // client does not implement.
            for unwanted in [
                extension::SUPPORTED_VERSIONS,
                extension::KEY_SHARE,
                extension::PRE_SHARED_KEY,
                35, /* session_ticket */
                extension::ALPN,
            ] {
                assert!(
                    !types.contains(&unwanted),
                    "extension {unwanted} must not be offered"
                );
            }
        });
    }

    #[test]
    fn an_ip_address_is_not_sent_as_a_server_name() {
        let ip = ServerName::Ip("192.0.2.7".parse().expect("ip"));
        parse(&client_hello(ip), |hello| {
            assert!(hello
                .extensions
                .iter()
                .all(|e| e.typ != extension::SERVER_NAME));
        });
    }

    #[test]
    fn two_hellos_never_share_a_random() {
        let a = client_hello(ServerName::Dns(SERVER));
        let b = client_hello(ServerName::Dns(SERVER));
        assert_ne!(a[11..43], b[11..43], "the client random must be fresh");
    }

    #[test]
    fn an_empty_suite_or_group_list_is_refused_at_the_start() {
        let anchors: [TrustAnchor<'_>; 0] = [];
        let no_suites = ClientConfig12 {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: &[NamedGroup::X25519],
            cipher_suites: &[],
        };
        assert!(ClientHandshake12::start(&no_suites).is_err());
        let no_groups = ClientConfig12 {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: &[],
            cipher_suites: CipherSuite12::SUPPORTED,
        };
        assert!(ClientHandshake12::start(&no_groups).is_err());
        // A value that is not a suite this client implements cannot be offered.
        let bogus = ClientConfig12 {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: &[NamedGroup::X25519],
            cipher_suites: &[CipherSuite12(0x002f)], // TLS_RSA_WITH_AES_128_CBC_SHA
        };
        assert_eq!(
            ClientHandshake12::start(&bogus).err(),
            Some(ClientError::UnofferedCipherSuite(0x002f))
        );
    }
}

// ===========================================================================
// The refusals
// ===========================================================================

/// A scripted TLS 1.2 server built from this crate's own primitives.
///
/// It plays a *correct* server by default and lets a test corrupt exactly one
/// thing. That would be circular if it were used to show the client works, so it
/// is not: it is used only to make the client **refuse**, and a refusal cannot
/// come from shared wrongness (if the server and the client agreed on a
/// malformed flight, the client would accept it and the test would fail). A
/// control test proves the uncorrupted flight completes, so no refusal test can
/// pass because the scripted server was simply broken.
mod fake {
    use super::*;
    use rusty_tls::handrolled::handshake::{
        messages, ClientHello, Extension, HandshakeType, Message,
    };
    use rusty_tls::handrolled::handshake12::{
        message, parse_client_key_exchange, Certificate12, CertificateRequest12, ServerHello12,
        ServerKeyExchange,
    };
    use rusty_tls::handrolled::kx::KeyExchange;
    use rusty_tls::handrolled::record::{Aead, ContentType};
    use rusty_tls::handrolled::record12::{Opener, Sealer};
    use rusty_tls::handrolled::schedule::Hash;
    use rusty_tls::handrolled::schedule12::{
        extended_master_secret, finished_verify_data, key_block, MasterSecret, Side,
    };
    use rusty_tls::handrolled::sign::SigningKey;
    use rusty_tls::handrolled::wire::Writer;

    pub const TLS12: u16 = 0x0303;

    /// A server's ServerKeyExchange, field by field, so a test can change any.
    #[derive(Clone)]
    pub struct Ske {
        pub curve_type: u8,
        pub curve: u16,
        pub public: Vec<u8>,
        pub scheme: u16,
        pub signature: Vec<u8>,
    }

    /// The server's first flight, field by field.
    pub struct Flight1 {
        pub version: u16,
        pub random: [u8; 32],
        pub session_id: Vec<u8>,
        pub suite: u16,
        pub compression: u8,
        /// `None` omits the extensions block entirely, as old servers do.
        pub extensions: Option<Vec<(u16, Vec<u8>)>>,
        pub certificates: Vec<Vec<u8>>,
        pub ske: Option<Ske>,
        pub certificate_request: bool,
        pub hello_done_body: Vec<u8>,
        pub client_random: [u8; 32],
    }

    impl Flight1 {
        /// The messages, in the order a correct server sends them.
        pub fn messages(&self) -> Vec<Vec<u8>> {
            let extensions: Option<Vec<Extension<'_>>> = self.extensions.as_ref().map(|list| {
                list.iter()
                    .map(|(typ, data)| Extension { typ: *typ, data })
                    .collect()
            });
            let mut out = Vec::new();

            let hello = ServerHello12 {
                version: self.version,
                random: &self.random,
                session_id: &self.session_id,
                cipher_suite: self.suite,
                compression: self.compression,
                extensions: extensions.clone().unwrap_or_default(),
            };
            let mut body = hello.encode();
            // `encode` omits an empty block, which is also what `None` means; an
            // explicitly empty `Some(vec![])` should write a zero-length block.
            if matches!(&self.extensions, Some(list) if list.is_empty()) {
                body.extend_from_slice(&[0, 0]);
            }
            out.push(message(HandshakeType::ServerHello, &body));

            let chain: Vec<&[u8]> = self.certificates.iter().map(Vec::as_slice).collect();
            out.push(message(
                HandshakeType::Certificate,
                &Certificate12::encode(&chain),
            ));

            if let Some(ske) = &self.ske {
                let mut w = Writer::new();
                w.u8(ske.curve_type);
                w.u16(ske.curve);
                w.vector_u8(|w| w.bytes(&ske.public));
                w.u16(ske.scheme);
                w.vector_u16(|w| w.bytes(&ske.signature));
                out.push(message(HandshakeType::ServerKeyExchange, &w.into_vec()));
            }
            if self.certificate_request {
                let request = CertificateRequest12 {
                    certificate_types: &[64, 1],
                    signature_algorithms: vec![0x0403, 0x0804],
                    authorities: &[],
                };
                out.push(message(
                    HandshakeType::CertificateRequest,
                    &request.encode(),
                ));
            }
            out.push(message(
                HandshakeType::ServerHelloDone,
                &self.hello_done_body,
            ));
            out
        }
    }

    /// How the server's final flight is corrupted, if at all.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Finish {
        Good,
        /// One bit of `verify_data` flipped.
        WrongVerifyData,
        /// Computed with the client's label, i.e. reflecting the client's Finished.
        ReflectedClientLabel,
        /// Encrypted under a key the client never derived.
        WrongKey,
        /// No ChangeCipherSpec before the Finished.
        NoChangeCipherSpec,
        /// The Finished sent in the clear after the ChangeCipherSpec.
        Plaintext,
        /// Eleven octets of `verify_data`.
        ShortVerifyData,
        /// A second handshake message in the same record as the Finished.
        TrailingMessage,
        /// The Finished split across two protected records.
        SplitAcrossRecords,
        /// A ChangeCipherSpec whose body is `0x02`.
        CcsValueTwo,
        /// A ChangeCipherSpec with no body.
        CcsEmpty,
        /// A ChangeCipherSpec with a two-octet body.
        CcsTwoOctets,
        /// A second ChangeCipherSpec after the first.
        CcsTwice,
        /// A fatal alert, protected, instead of the Finished.
        FatalAlert,
        /// A one-octet alert, protected, instead of the Finished.
        MalformedAlert,
    }

    pub struct Fake {
        signer: SigningKey,
        scheme: u16,
        leaf_der: Vec<u8>,
        chain: Vec<Vec<u8>>,
        kx: Option<KeyExchange>,
        pub transcript: Vec<u8>,
        hash: Hash,
        aead: Aead,
        client_random: [u8; 32],
        server_random: [u8; 32],
        master: Option<MasterSecret>,
        pub sealer: Option<Sealer>,
        pub opener: Option<Opener>,
        /// Whether the client's Finished verified on this side.
        pub client_finished_ok: Option<bool>,
        /// The handshake message types in the client's second flight.
        pub client_types: Vec<HandshakeType>,
    }

    pub fn signer_for(leaf: Leaf, pkcs8: &[u8]) -> (SigningKey, u16) {
        match leaf {
            Leaf::P256 => (SigningKey::ecdsa_p256(pkcs8).expect("p256"), 0x0403),
            Leaf::P384 => (SigningKey::ecdsa_p384(pkcs8).expect("p384"), 0x0503),
            Leaf::Ed25519 => (SigningKey::ed25519(pkcs8).expect("ed25519"), 0x0807),
            Leaf::Rsa => (SigningKey::rsa(pkcs8).expect("rsa"), 0x0804),
        }
    }

    pub fn suite_for(leaf: Leaf, offered: &[u16]) -> u16 {
        let ecdsa = leaf != Leaf::Rsa;
        let want: &[u16] = if ecdsa {
            &[0xc02b, 0xc02c, 0xcca9]
        } else {
            &[0xc02f, 0xc030, 0xcca8]
        };
        *offered
            .iter()
            .find(|s| want.contains(s))
            .expect("the client offered a suite for this key")
    }

    impl Fake {
        pub fn new(pki: &Pki, leaf: Leaf) -> Self {
            let (signer, scheme) = signer_for(leaf, &pki.leaf_pkcs8);
            Self {
                signer,
                scheme,
                leaf_der: pki.leaf_der.clone(),
                chain: pki.chain.iter().map(|c| c.as_ref().to_vec()).collect(),
                kx: None,
                transcript: Vec::new(),
                hash: Hash::Sha256,
                aead: Aead::Aes128Gcm,
                client_random: [0; 32],
                server_random: [0; 32],
                master: None,
                sealer: None,
                opener: None,
                client_finished_ok: None,
                client_types: Vec::new(),
            }
        }

        pub fn signer(&self) -> &SigningKey {
            &self.signer
        }

        /// A correct first flight in answer to the client's hello record.
        pub fn flight1(&mut self, hello_record: &[u8]) -> Flight1 {
            let msgs = messages(&hello_record[5..]).expect("one message");
            assert_eq!(msgs[0].typ, HandshakeType::ClientHello);
            self.transcript.extend_from_slice(msgs[0].encoded);
            let hello = ClientHello::parse(msgs[0].body).expect("hello parses");
            self.client_random.copy_from_slice(hello.random);

            let suite = suite_for(
                if self.scheme == 0x0804 {
                    Leaf::Rsa
                } else {
                    Leaf::P256
                },
                &hello.cipher_suites,
            );
            let (aead, hash, _) = CipherSuite12(suite).parts().expect("known suite");
            self.aead = aead;
            self.hash = hash;
            self.server_random = [0x5a; 32];
            self.server_random[..8].copy_from_slice(
                &std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
                    .to_be_bytes()[8..],
            );

            let kx = KeyExchange::generate(NamedGroup::SecP256R1).expect("kx");
            let params =
                ServerKeyExchange::encode_params(NamedGroup::SecP256R1.as_u16(), kx.public_key());
            let signed = rusty_tls::handrolled::handshake12::signed_content(
                &self.client_random,
                &self.server_random,
                &params,
            );
            let signature = self
                .signer
                .sign(
                    rusty_tls::handrolled::verify::SignatureScheme(self.scheme),
                    &signed,
                )
                .expect("signs");
            let ske = Ske {
                curve_type: 3,
                curve: NamedGroup::SecP256R1.as_u16(),
                public: kx.public_key().to_vec(),
                scheme: self.scheme,
                signature,
            };
            self.kx = Some(kx);

            Flight1 {
                version: TLS12,
                random: self.server_random,
                session_id: Vec::new(),
                suite,
                compression: 0,
                extensions: Some(vec![(23, Vec::new()), (0xff01, vec![0]), (11, vec![1, 0])]),
                certificates: self.chain.iter().take(1).cloned().collect(),
                ske: Some(ske),
                certificate_request: false,
                hello_done_body: Vec::new(),
                client_random: self.client_random,
            }
        }

        /// The leaf certificate this server presents.
        pub fn leaf_der(&self) -> &[u8] {
            &self.leaf_der
        }

        /// Record `sent` as the transcript of the server's flight.
        pub fn sent(&mut self, sent: &[Vec<u8>]) {
            for message in sent {
                self.transcript.extend_from_slice(message);
            }
        }

        /// Take the client's second flight: derive keys, check its Finished.
        pub fn client_flight(&mut self, flight: &[u8]) {
            let mut offset = 0usize;
            let mut records = Vec::new();
            while offset < flight.len() {
                let len =
                    5 + usize::from(u16::from_be_bytes([flight[offset + 3], flight[offset + 4]]));
                records.push(&flight[offset..offset + len]);
                offset += len;
            }
            assert_eq!(
                records.len(),
                3,
                "client flight: handshake, ChangeCipherSpec, Finished"
            );
            assert_eq!(records[0][0], 22);
            assert_eq!(
                records[1],
                [20, 3, 3, 0, 1, 1],
                "a one-octet ChangeCipherSpec"
            );

            let msgs: Vec<Message<'_>> = messages(&records[0][5..]).expect("messages");
            let cke = msgs
                .iter()
                .find(|m| m.typ == HandshakeType::ClientKeyExchange)
                .expect("a ClientKeyExchange");
            for m in &msgs {
                self.transcript.extend_from_slice(m.encoded);
            }
            self.client_types = msgs.iter().map(|m| m.typ).collect();
            let client_public = parse_client_key_exchange(cke.body)
                .expect("parses")
                .to_vec();

            let pms = self
                .kx
                .take()
                .expect("server key")
                .agree(&client_public, |s| s.to_vec())
                .expect("agrees");
            let session_hash = self.hash.hash(&self.transcript);
            let master = extended_master_secret(self.hash, &pms, &session_hash).expect("ems");
            let kb = key_block(
                self.hash,
                self.aead,
                &master,
                &self.client_random,
                &self.server_random,
            );
            self.sealer = Some(
                Sealer::new(self.aead, &kb.server_write_key, &kb.server_write_iv).expect("sealer"),
            );
            let mut opener =
                Opener::new(self.aead, &kb.client_write_key, &kb.client_write_iv).expect("opener");

            let opened = opener
                .open(records[2])
                .expect("the client's Finished decrypts");
            let expected = finished_verify_data(
                self.hash,
                &master,
                Side::Client,
                &self.hash.hash(&self.transcript),
            )
            .expect("fin");
            self.client_finished_ok = Some(opened.fragment[4..] == expected);
            self.transcript.extend_from_slice(&opened.fragment);
            self.opener = Some(opener);
            self.master = Some(master);
        }

        /// The server's final flight, as records.
        pub fn finish(&mut self, mode: Finish) -> Vec<Vec<u8>> {
            let master = self.master.as_ref().expect("keys derived");
            let side = if mode == Finish::ReflectedClientLabel {
                Side::Client
            } else {
                Side::Server
            };
            let mut verify =
                finished_verify_data(self.hash, master, side, &self.hash.hash(&self.transcript))
                    .expect("verify_data")
                    .to_vec();
            if mode == Finish::WrongVerifyData {
                verify[0] ^= 1;
            }
            if mode == Finish::ShortVerifyData {
                verify.truncate(11);
            }
            let finished = message(HandshakeType::Finished, &verify);
            let ccs = match mode {
                Finish::CcsValueTwo => vec![20, 3, 3, 0, 1, 2],
                Finish::CcsEmpty => vec![20, 3, 3, 0, 0],
                Finish::CcsTwoOctets => vec![20, 3, 3, 0, 2, 1, 1],
                _ => vec![20, 3, 3, 0, 1, 1],
            };
            let sealer = self.sealer.as_mut().expect("sealer");

            let mut out = Vec::new();
            if mode != Finish::NoChangeCipherSpec {
                out.push(ccs.clone());
            }
            if mode == Finish::CcsTwice {
                out.push(ccs);
            }
            match mode {
                Finish::FatalAlert => {
                    out.push(sealer.seal(ContentType::Alert, &[2, 40]).expect("seals"))
                }
                Finish::MalformedAlert => {
                    out.push(sealer.seal(ContentType::Alert, &[2]).expect("seals"))
                }
                Finish::Plaintext => {
                    let mut record = vec![22, 3, 3];
                    record.extend_from_slice(&(finished.len() as u16).to_be_bytes());
                    record.extend_from_slice(&finished);
                    out.push(record);
                }
                Finish::WrongKey => {
                    let mut other = Sealer::new(
                        self.aead,
                        &vec![9u8; self.aead.key_len()],
                        &vec![9u8; rusty_tls::handrolled::record12::fixed_iv_len(self.aead)],
                    )
                    .expect("sealer");
                    out.push(
                        other
                            .seal(ContentType::Handshake, &finished)
                            .expect("seals"),
                    );
                }
                Finish::TrailingMessage => {
                    let mut fragment = finished.clone();
                    fragment.extend_from_slice(&[0, 0, 0, 0]); // a HelloRequest
                    out.push(
                        sealer
                            .seal(ContentType::Handshake, &fragment)
                            .expect("seals"),
                    );
                }
                Finish::SplitAcrossRecords => {
                    let (a, b) = finished.split_at(7);
                    out.push(sealer.seal(ContentType::Handshake, a).expect("seals"));
                    out.push(sealer.seal(ContentType::Handshake, b).expect("seals"));
                }
                _ => out.push(
                    sealer
                        .seal(ContentType::Handshake, &finished)
                        .expect("seals"),
                ),
            }
            out
        }

        /// Application data from the server.
        pub fn send(&mut self, data: &[u8]) -> Vec<u8> {
            self.sealer
                .as_mut()
                .expect("sealer")
                .seal(ContentType::ApplicationData, data)
                .expect("seals")
        }

        /// Any record type from the server, protected.
        pub fn send_as(&mut self, typ: ContentType, fragment: &[u8]) -> Vec<u8> {
            self.sealer
                .as_mut()
                .expect("sealer")
                .seal(typ, fragment)
                .expect("seals")
        }

        /// Open a record the client sent.
        pub fn open(&mut self, record: &[u8]) -> (ContentType, Vec<u8>) {
            let opened = self
                .opener
                .as_mut()
                .expect("opener")
                .open(record)
                .expect("opens");
            (opened.typ, opened.fragment)
        }
    }

    /// Frame handshake messages as plaintext records, per `split`.
    pub fn frame(messages: &[Vec<u8>], split: Split) -> Vec<Vec<u8>> {
        let record = |fragment: &[u8]| {
            let mut r = vec![22, 3, 3];
            r.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
            r.extend_from_slice(fragment);
            r
        };
        match split {
            Split::PerMessage => messages.iter().map(|m| record(m)).collect(),
            Split::Coalesced => vec![record(&messages.concat())],
            Split::OneOctet => messages.concat().chunks(1).map(record).collect(),
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Split {
        PerMessage,
        /// Every message in one record.
        Coalesced,
        /// One octet per record: every message boundary falls mid-record.
        OneOctet,
    }
}

use fake::{Fake, Finish, Flight1, Split};

/// A hook that edits the server's first flight before it is serialised.
type FlightHook<'a> = Box<dyn FnMut(&mut Flight1, &Fake) + 'a>;
/// A hook that edits a list of messages or records.
type ListHook<'a> = Box<dyn FnMut(&mut Vec<Vec<u8>>) + 'a>;

/// What a test can change about the conversation.
struct Script<'a> {
    leaf: Leaf,
    /// Edit the server's first flight before it is serialised.
    flight: FlightHook<'a>,
    /// Edit the serialised messages (drop, reorder, add).
    messages: ListHook<'a>,
    /// Edit the framed records (inject an alert or a ChangeCipherSpec).
    records: ListHook<'a>,
    /// Edit the server's final records (ChangeCipherSpec and Finished).
    final_records: ListHook<'a>,
    /// Trust a different CA than the one that issued the served certificate.
    trust: Option<&'a Pki>,
    split: Split,
    finish: Finish,
    /// Change the client's configuration.
    suites: &'a [CipherSuite12],
    groups: &'a [NamedGroup],
    name: &'a str,
}

impl<'a> Script<'a> {
    fn new(leaf: Leaf) -> Self {
        Self {
            leaf,
            flight: Box::new(|_, _| {}),
            messages: Box::new(|_| {}),
            records: Box::new(|_| {}),
            final_records: Box::new(|_| {}),
            trust: None,
            split: Split::PerMessage,
            finish: Finish::Good,
            suites: CipherSuite12::SUPPORTED,
            groups: &[
                NamedGroup::X25519,
                NamedGroup::SecP256R1,
                NamedGroup::SecP384R1,
            ],
            name: SERVER,
        }
    }
    fn flight(mut self, f: impl FnMut(&mut Flight1, &Fake) + 'a) -> Self {
        self.flight = Box::new(f);
        self
    }
    fn messages(mut self, f: impl FnMut(&mut Vec<Vec<u8>>) + 'a) -> Self {
        self.messages = Box::new(f);
        self
    }
    fn records(mut self, f: impl FnMut(&mut Vec<Vec<u8>>) + 'a) -> Self {
        self.records = Box::new(f);
        self
    }
    fn final_records(mut self, f: impl FnMut(&mut Vec<Vec<u8>>) + 'a) -> Self {
        self.final_records = Box::new(f);
        self
    }
    fn trusting(mut self, pki: &'a Pki) -> Self {
        self.trust = Some(pki);
        self
    }
    fn groups(mut self, g: &'a [NamedGroup]) -> Self {
        self.groups = g;
        self
    }
    fn split(mut self, s: Split) -> Self {
        self.split = s;
        self
    }
    fn finish(mut self, f: Finish) -> Self {
        self.finish = f;
        self
    }
    fn name(mut self, n: &'a str) -> Self {
        self.name = n;
        self
    }
    fn suites(mut self, s: &'a [CipherSuite12]) -> Self {
        self.suites = s;
        self
    }
}

/// Run the whole conversation, returning the client's outcome and the server.
fn converse(pki: &Pki, mut script: Script<'_>) -> (Result<Connection12, ClientError>, Fake) {
    let anchors = [anchor(&script.trust.unwrap_or(pki).root_der)];
    let cfg = ClientConfig12 {
        server_name: ServerName::Dns(script.name),
        anchors: &anchors,
        path: options(),
        groups: script.groups,
        cipher_suites: script.suites,
    };
    let mut fake = Fake::new(pki, script.leaf);
    let (mut client, hello) = ClientHandshake12::start(&cfg).expect("starts");

    let mut flight = fake.flight1(&hello);
    (script.flight)(&mut flight, &fake);
    let mut sent = flight.messages();
    (script.messages)(&mut sent);
    fake.sent(&sent);
    let mut records = fake::frame(&sent, script.split);
    (script.records)(&mut records);

    let mut reply = Vec::new();
    for record in &records {
        match client.read_record(record) {
            Ok(bytes) => reply.extend_from_slice(&bytes),
            Err(err) => return (Err(err), fake),
        }
    }
    if reply.is_empty() {
        return (
            Err(ClientError::Failed), // the flight ended without the client replying
            fake,
        );
    }

    fake.client_flight(&reply);
    let mut last = fake.finish(script.finish);
    (script.final_records)(&mut last);
    for record in last {
        if let Err(err) = client.read_record(&record) {
            return (Err(err), fake);
        }
    }
    if !client.is_finished() {
        return (Err(ClientError::Failed), fake);
    }
    (client.into_connection(), fake)
}

fn refused(pki: &Pki, script: Script<'_>) -> ClientError {
    match converse(pki, script).0 {
        Err(err) => err,
        Ok(_) => panic!("the client completed a handshake it should have refused"),
    }
}

fn pki_p256() -> Pki {
    pki(Leaf::P256, SERVER)
}

// --------------------------------------------------------------- the control

/// The scripted server's *correct* flight completes, and its application data
/// round-trips. Without this, every refusal below could pass because the script
/// was broken.
#[test]
fn the_scripted_servers_correct_flight_completes() {
    for leaf in [Leaf::P256, Leaf::Rsa] {
        let pki = pki(leaf, SERVER);
        let (result, mut fake) = converse(&pki, Script::new(leaf));
        let mut connection = result.unwrap_or_else(|e| panic!("{leaf:?}: {e}"));
        assert_eq!(
            fake.client_finished_ok,
            Some(true),
            "the client's Finished verified at the server"
        );

        let record = connection.write(b"to the server").expect("writes");
        assert_eq!(
            fake.open(&record),
            (
                rusty_tls::handrolled::record::ContentType::ApplicationData,
                b"to the server".to_vec()
            )
        );
        let incoming = connection
            .read(&fake.send(b"to the client"))
            .expect("reads");
        assert_eq!(incoming, Incoming12::Application(b"to the client".to_vec()));
    }
}

/// The same flight delivered as one coalesced record, as one octet per record,
/// and with the server's Finished split across two protected records, all
/// complete: reassembly crosses every message boundary.
#[test]
fn a_flight_is_reassembled_however_it_is_framed() {
    let pki = pki_p256();
    for split in [Split::PerMessage, Split::Coalesced, Split::OneOctet] {
        let (result, _) = converse(&pki, Script::new(Leaf::P256).split(split));
        assert!(result.is_ok(), "{split:?}: {:?}", result.err());
    }
    let (result, _) = converse(
        &pki,
        Script::new(Leaf::P256).finish(Finish::SplitAcrossRecords),
    );
    assert!(
        result.is_ok(),
        "a Finished split across records: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------- refusals

mod refusals {
    use super::*;
    use rusty_tls::handrolled::client::{Alert, AlertDescription, AlertLevel};
    use rusty_tls::handrolled::handshake::{HandshakeError, HandshakeType};
    use rusty_tls::handrolled::handshake12::{signed_content, ServerKeyExchange};
    use rusty_tls::handrolled::kx::KxError;
    use rusty_tls::handrolled::name::NameError;
    use rusty_tls::handrolled::path::PathError;
    use rusty_tls::handrolled::record::{ContentType, RecordError};
    use rusty_tls::handrolled::sign::SigningKey;
    use rusty_tls::handrolled::verify::{SignatureScheme, VerifyError};

    /// Re-sign the ServerKeyExchange after a test changed what it covers.
    fn resign(f: &mut Flight1, fake: &Fake, client_random: &[u8], server_random: &[u8]) {
        let ske = f.ske.as_mut().expect("a ServerKeyExchange");
        let params = ServerKeyExchange::encode_params(ske.curve, &ske.public);
        let signed = signed_content(client_random, server_random, &params);
        ske.signature = fake
            .signer()
            .sign(SignatureScheme(ske.scheme), &signed)
            .expect("signs");
    }

    fn alert_record(level: u8, description: u8) -> Vec<u8> {
        vec![21, 3, 3, 0, 2, level, description]
    }

    fn peer_alert(description: u8, level: AlertLevel) -> ClientError {
        ClientError::PeerAlert(Alert {
            level,
            description: AlertDescription(description),
        })
    }

    // -------------------------------------------------------- the ServerHello

    #[test]
    fn a_server_that_selects_another_version_is_refused() {
        let pki = pki_p256();
        for version in [0x0300u16, 0x0301, 0x0302, 0x0304, 0x0305, 0x0000] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| f.version = version),
            );
            assert_eq!(err, ClientError::NotTls12(version), "0x{version:04x}");
        }
    }

    #[test]
    fn a_cipher_suite_that_was_not_offered_is_refused() {
        let pki = pki_p256();
        // TLS 1.3, static RSA, CBC, DHE, a SCSV, the null suite, and nonsense.
        for suite in [
            0x1301u16, 0x002f, 0xc013, 0x009c, 0x0033, 0x00ff, 0x0000, 0xffff,
        ] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| f.suite = suite),
            );
            assert_eq!(
                err,
                ClientError::UnofferedCipherSuite(suite),
                "0x{suite:04x}"
            );
        }
    }

    #[test]
    fn a_suite_this_configuration_did_not_offer_is_refused() {
        // The server picks the ChaCha suite; the client offered only AES-128.
        let pki = pki_p256();
        let only_aes = [CipherSuite12::ECDHE_ECDSA_WITH_AES_128_GCM_SHA256];
        let err = refused(
            &pki,
            Script::new(Leaf::P256)
                .suites(&only_aes)
                .flight(|f, _| f.suite = 0xcca9),
        );
        assert_eq!(err, ClientError::UnofferedCipherSuite(0xcca9));
    }

    #[test]
    fn a_compression_method_is_refused() {
        let pki = pki_p256();
        for method in [1u8, 2, 0xff] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| f.compression = method),
            );
            assert_eq!(
                err,
                ClientError::Handshake(HandshakeError::UnexpectedCompression),
                "{method}"
            );
        }
    }

    #[test]
    fn a_server_without_the_extended_master_secret_is_refused() {
        let pki = pki_p256();
        for extensions in [
            // absent
            Some(vec![(0xff01u16, vec![0u8]), (11, vec![1, 0])]),
            // present with a body
            Some(vec![(23, vec![0]), (0xff01, vec![0]), (11, vec![1, 0])]),
            // the whole block omitted, as a pre-extensions server does
            None,
            // an empty block
            Some(vec![]),
        ] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| f.extensions = extensions.clone()),
            );
            assert_eq!(err, ClientError::MissingExtendedMasterSecret);
        }
    }

    #[test]
    fn a_server_without_secure_renegotiation_is_refused() {
        let pki = pki_p256();
        for renegotiation in [
            None,
            Some(vec![]),
            Some(vec![1, 2]),
            Some(vec![0, 0]),
            Some(vec![5]),
        ] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| {
                    let mut list = vec![(23u16, vec![]), (11, vec![1, 0])];
                    if let Some(body) = renegotiation.clone() {
                        list.push((0xff01, body));
                    }
                    f.extensions = Some(list);
                }),
            );
            assert_eq!(err, ClientError::BadRenegotiationInfo);
        }
    }

    #[test]
    fn an_extension_the_client_did_not_offer_is_refused() {
        let pki = pki_p256();
        // session_ticket, ALPN, supported_versions, heartbeat, signature_algorithms,
        // key_share, and an unassigned value. (`supported_groups` is not here:
        // RFC 8422 lets a server volunteer its groups, and it is ignored; see
        // `a_servers_supported_groups_are_tolerated`.)
        for typ in [35u16, 16, 43, 15, 13, 51, 0xfffe] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| {
                    f.extensions.as_mut().expect("block").push((typ, vec![0]));
                }),
            );
            assert_eq!(err, ClientError::UnofferedExtension(typ), "{typ}");
        }
    }

    /// BoGo SupportedCurves-ServerHello-TLS12.
    #[test]
    fn a_servers_supported_groups_are_tolerated() {
        let pki = pki_p256();
        let (result, _) = converse(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| {
                f.extensions
                    .as_mut()
                    .expect("block")
                    .push((10, vec![0, 2, 0, 0x17]));
            }),
        );
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn a_duplicated_extension_is_refused() {
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256)
                .flight(|f, _| f.extensions.as_mut().expect("block").push((23, vec![]))),
        );
        assert_eq!(
            err,
            ClientError::Handshake(HandshakeError::DuplicateExtension(23))
        );
    }

    #[test]
    fn point_formats_must_include_uncompressed() {
        let pki = pki_p256();
        for formats in [
            vec![1u8, 1],
            vec![2, 1, 2],
            vec![0],
            vec![2, 0],
            vec![3, 0, 1],
        ] {
            let label = format!("{formats:?}");
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| {
                    let list = f.extensions.as_mut().expect("block");
                    list.retain(|(t, _)| *t != 11);
                    list.push((11, formats.clone()));
                }),
            );
            assert_eq!(err, ClientError::UnsupportedPointFormat, "{label}");
        }
        // Uncompressed alongside others is fine, as is an absent extension.
        for formats in [Some(vec![3u8, 0, 1, 2]), Some(vec![1, 0]), None] {
            let (result, _) = converse(
                &pki,
                Script::new(Leaf::P256).flight(move |f, _| {
                    let list = f.extensions.as_mut().expect("block");
                    list.retain(|(t, _)| *t != 11);
                    if let Some(body) = formats.clone() {
                        list.push((11, body));
                    }
                }),
            );
            assert!(result.is_ok(), "{:?}", result.err());
        }
    }

    #[test]
    fn a_server_name_acknowledgement_is_fine_but_not_with_data() {
        let pki = pki_p256();
        let (ok, _) = converse(
            &pki,
            Script::new(Leaf::P256)
                .flight(|f, _| f.extensions.as_mut().expect("block").push((0, vec![]))),
        );
        assert!(ok.is_ok(), "{:?}", ok.err());
        let err = refused(
            &pki,
            Script::new(Leaf::P256)
                .flight(|f, _| f.extensions.as_mut().expect("block").push((0, vec![1]))),
        );
        assert_eq!(
            err,
            ClientError::Handshake(HandshakeError::Malformed(
                "a server_name acknowledgement is not empty"
            ))
        );
    }

    #[test]
    fn a_session_id_is_tolerated_up_to_thirty_two_octets_and_never_resumed() {
        let pki = pki_p256();
        let (ok, _) = converse(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| f.session_id = vec![7; 32]),
        );
        assert!(ok.is_ok(), "{:?}", ok.err());
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| f.session_id = vec![7; 33]),
        );
        assert_eq!(
            err,
            ClientError::Handshake(HandshakeError::Malformed("session_id is over 32 octets"))
        );
    }

    // ----------------------------------------------------------- Certificate

    #[test]
    fn an_empty_or_unparseable_certificate_chain_is_refused() {
        let pki = pki_p256();
        assert_eq!(
            refused(
                &pki,
                Script::new(Leaf::P256).flight(|f, _| f.certificates.clear())
            ),
            ClientError::NoCertificates
        );
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| f.certificates = vec![vec![1, 2, 3]]),
        );
        assert!(matches!(err, ClientError::MalformedCertificate(_)), "{err}");
    }

    #[test]
    fn a_chain_to_an_untrusted_root_is_refused() {
        let served = pki_p256();
        let trusted = pki_p256(); // a different CA
        let err = refused(&served, Script::new(Leaf::P256).trusting(&trusted));
        assert!(matches!(err, ClientError::Path(_)), "{err}");
    }

    #[test]
    fn a_certificate_for_another_name_is_refused() {
        let pki = pki_p256();
        let err = refused(&pki, Script::new(Leaf::P256).name("somewhere.else.example"));
        assert!(
            matches!(
                err,
                ClientError::Path(PathError::Name(NameError::NoMatchingSubjectAltName))
            ),
            "{err}"
        );
    }

    #[test]
    fn an_expired_or_not_yet_valid_certificate_is_refused() {
        let expired = pki_with(Leaf::P256, SERVER, |p| {
            p.not_before = OffsetDateTime::from_unix_timestamp(1_400_000_000).expect("t");
            p.not_after = OffsetDateTime::from_unix_timestamp(1_500_000_000).expect("t");
        });
        assert!(matches!(
            refused(&expired, Script::new(Leaf::P256)),
            ClientError::Path(PathError::Expired { .. })
        ));

        let future = pki_with(Leaf::P256, SERVER, |p| {
            p.not_before = OffsetDateTime::from_unix_timestamp(1_900_000_000).expect("t");
            p.not_after = OffsetDateTime::from_unix_timestamp(2_000_000_000).expect("t");
        });
        assert!(matches!(
            refused(&future, Script::new(Leaf::P256)),
            ClientError::Path(PathError::NotYetValid { .. })
        ));
    }

    #[test]
    fn a_certificate_that_cannot_authenticate_the_negotiated_suite_is_refused() {
        // An ECDSA certificate under an ECDHE_RSA suite, and the reverse.
        let ec = pki(Leaf::P256, SERVER);
        let err = refused(&ec, Script::new(Leaf::P256).flight(|f, _| f.suite = 0xc02f));
        assert_eq!(err, ClientError::KeyTypeMismatch);

        let rsa = pki(Leaf::Rsa, SERVER);
        let err = refused(&rsa, Script::new(Leaf::Rsa).flight(|f, _| f.suite = 0xc02b));
        assert_eq!(err, ClientError::KeyTypeMismatch);
    }

    // ------------------------------------------------------ ServerKeyExchange

    #[test]
    fn a_tampered_key_exchange_signature_is_refused() {
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| {
                let sig = &mut f.ske.as_mut().expect("ske").signature;
                let last = sig.len() - 1;
                sig[last] ^= 1;
            }),
        );
        assert_eq!(err, ClientError::Verify(VerifyError::BadSignature));
    }

    /// The signature must cover both randoms: otherwise a captured
    /// ServerKeyExchange could be replayed into another handshake.
    #[test]
    fn a_signature_that_does_not_cover_both_randoms_is_refused() {
        let pki = pki_p256();
        // Signed over a different client random.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, fake| {
                let server_random = f.random;
                resign(f, fake, &[0u8; 32], &server_random);
            }),
        );
        assert_eq!(
            err,
            ClientError::Verify(VerifyError::BadSignature),
            "wrong client random"
        );

        // Signed over a different server random (a replay from another session).
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, fake| {
                let client_random = f.client_random;
                resign(f, fake, &client_random, &[0u8; 32]);
            }),
        );
        assert_eq!(
            err,
            ClientError::Verify(VerifyError::BadSignature),
            "wrong server random"
        );
    }

    #[test]
    fn a_signature_that_does_not_cover_the_curve_is_refused() {
        // The server relabels the curve after signing. The label is covered, so
        // the signature fails rather than the client using a curve nobody chose.
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| {
                f.ske.as_mut().expect("ske").curve = NamedGroup::SecP384R1.as_u16();
            }),
        );
        assert_eq!(err, ClientError::Verify(VerifyError::BadSignature));
    }

    /// An attacker who copied the certificate off the wire but holds a different
    /// key. The certificate proves nothing without this signature.
    #[test]
    fn a_key_exchange_signed_by_a_key_other_than_the_certificates_is_refused() {
        let pki = pki_p256();
        let other = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("key");
        let attacker = SigningKey::ecdsa_p256(&other.serialize_der()).expect("signer");
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(move |f, _| {
                let ske = f.ske.as_mut().expect("ske");
                let params = ServerKeyExchange::encode_params(ske.curve, &ske.public);
                let signed = signed_content(&f.client_random, &f.random, &params);
                ske.signature = attacker
                    .sign(SignatureScheme(ske.scheme), &signed)
                    .expect("signs");
            }),
        );
        assert_eq!(err, ClientError::Verify(VerifyError::BadSignature));
    }

    #[test]
    fn an_unoffered_or_unusable_curve_is_refused() {
        let pki = pki_p256();
        // secp521r1, secp256k1, an unassigned value, and x448.
        for curve in [0x0019u16, 0x0016, 0x00ff, 0x001e] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256)
                    .flight(move |f, _| f.ske.as_mut().expect("ske").curve = curve),
            );
            assert_eq!(err, ClientError::UnofferedGroup(curve), "0x{curve:04x}");
        }
        // A real curve, but not one this client offered.
        let only_x25519 = [NamedGroup::X25519];
        let err = refused(&pki, Script::new(Leaf::P256).groups(&only_x25519));
        assert_eq!(
            err,
            ClientError::UnofferedGroup(NamedGroup::SecP256R1.as_u16())
        );
    }

    #[test]
    fn explicit_curve_parameters_are_refused() {
        let pki = pki_p256();
        for curve_type in [1u8, 2, 0, 4, 0xff] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256)
                    .flight(move |f, _| f.ske.as_mut().expect("ske").curve_type = curve_type),
            );
            assert_eq!(
                err,
                ClientError::Handshake(HandshakeError::UnexpectedCurveType(curve_type))
            );
        }
    }

    #[test]
    fn a_signature_scheme_that_was_not_offered_is_refused() {
        let pki = pki_p256();
        // SHA-1 RSA and ECDSA, secp521r1+sha512, ed448, and nonsense.
        for scheme in [0x0201u16, 0x0203, 0x0603, 0x0808, 0x0000, 0xffff] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256)
                    .flight(move |f, _| f.ske.as_mut().expect("ske").scheme = scheme),
            );
            assert_eq!(
                err,
                ClientError::UnofferedSignatureScheme(scheme),
                "0x{scheme:04x}"
            );
        }
    }

    #[test]
    fn a_scheme_for_the_wrong_kind_of_key_is_refused() {
        let ec = pki(Leaf::P256, SERVER);
        for scheme in [0x0401u16, 0x0804, 0x0807] {
            let err = refused(
                &ec,
                Script::new(Leaf::P256)
                    .flight(move |f, _| f.ske.as_mut().expect("ske").scheme = scheme),
            );
            assert_eq!(
                err,
                ClientError::Verify(VerifyError::KeyAlgorithmMismatch),
                "ec key, 0x{scheme:04x}"
            );
        }
        let rsa = pki(Leaf::Rsa, SERVER);
        for scheme in [0x0403u16, 0x0503, 0x0807] {
            let err = refused(
                &rsa,
                Script::new(Leaf::Rsa)
                    .flight(move |f, _| f.ske.as_mut().expect("ske").scheme = scheme),
            );
            assert_eq!(
                err,
                ClientError::Verify(VerifyError::KeyAlgorithmMismatch),
                "rsa key, 0x{scheme:04x}"
            );
        }
    }

    #[test]
    fn an_empty_signature_or_public_key_is_refused() {
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| f.ske.as_mut().expect("ske").signature.clear()),
        );
        assert!(
            matches!(err, ClientError::Handshake(HandshakeError::Empty(_))),
            "{err}"
        );
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| f.ske.as_mut().expect("ske").public.clear()),
        );
        assert!(
            matches!(err, ClientError::Handshake(HandshakeError::Empty(_))),
            "{err}"
        );
    }

    /// A correctly signed key exchange can still carry a public key that is not
    /// a point on the curve, or a low-order one. The agreement refuses it.
    #[test]
    fn an_invalid_server_public_key_is_refused() {
        let pki = pki_p256();
        for public in [vec![0x04; 65], vec![0u8; 65], vec![1, 2, 3], vec![0x04; 64]] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).flight(move |f, fake| {
                    f.ske.as_mut().expect("ske").public = public.clone();
                    let (client_random, server_random) = (f.client_random, f.random);
                    resign(f, fake, &client_random, &server_random);
                }),
            );
            assert_eq!(err, ClientError::Kx(KxError::BadPeerKey));
        }
        // The all-zero X25519 point is the one case X25519 itself must catch.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, fake| {
                let ske = f.ske.as_mut().expect("ske");
                ske.curve = NamedGroup::X25519.as_u16();
                ske.public = vec![0u8; 32];
                let (client_random, server_random) = (f.client_random, f.random);
                resign(f, fake, &client_random, &server_random);
            }),
        );
        assert_eq!(err, ClientError::Kx(KxError::BadPeerKey), "all-zero X25519");
    }

    // -------------------------------------------------------- message order

    fn unexpected(expected: &'static str, got: HandshakeType) -> ClientError {
        ClientError::UnexpectedMessage { expected, got }
    }

    #[test]
    fn a_missing_or_reordered_message_is_refused() {
        let pki = pki_p256();
        // [0] ServerHello [1] Certificate [2] ServerKeyExchange [3] ServerHelloDone
        let err = refused(
            &pki,
            Script::new(Leaf::P256).messages(|m| {
                m.remove(2);
            }),
        );
        assert_eq!(
            err,
            unexpected("ServerKeyExchange", HandshakeType::ServerHelloDone),
            "no ServerKeyExchange"
        );

        let err = refused(
            &pki,
            Script::new(Leaf::P256).messages(|m| {
                m.remove(1);
            }),
        );
        assert_eq!(
            err,
            unexpected("Certificate", HandshakeType::ServerKeyExchange),
            "no Certificate"
        );

        let err = refused(&pki, Script::new(Leaf::P256).messages(|m| m.swap(1, 2)));
        assert_eq!(
            err,
            unexpected("Certificate", HandshakeType::ServerKeyExchange),
            "swapped"
        );

        let err = refused(
            &pki,
            Script::new(Leaf::P256).messages(|m| {
                let done = m.pop().expect("m");
                m.insert(1, done);
            }),
        );
        assert_eq!(
            err,
            unexpected("Certificate", HandshakeType::ServerHelloDone),
            "done first"
        );
    }

    #[test]
    fn a_repeated_or_unsolicited_message_is_refused() {
        let pki = pki_p256();
        // A second ServerHello where the Certificate belongs.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).messages(|m| {
                let hello = m[0].clone();
                m.insert(1, hello);
            }),
        );
        assert_eq!(err, unexpected("Certificate", HandshakeType::ServerHello));

        // A HelloRequest before the ServerHello. RFC 5246 lets a client ignore
        // one; this client refuses, which is the conservative reading.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).messages(|m| m.insert(0, vec![0, 0, 0, 0])),
        );
        assert_eq!(err, unexpected("ServerHello", HandshakeType::HelloRequest));

        // A CertificateStatus (type 22) the client never asked for.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).messages(|m| m.insert(2, vec![22, 0, 0, 1, 0])),
        );
        assert_eq!(
            err,
            unexpected("ServerKeyExchange", HandshakeType::Unknown(22))
        );

        // A Finished before the flight is over.
        let err = refused(
            &pki,
            Script::new(Leaf::P256)
                .messages(|m| m.insert(3, vec![20, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
        );
        assert_eq!(
            err,
            unexpected(
                "CertificateRequest or ServerHelloDone",
                HandshakeType::Finished
            )
        );
    }

    #[test]
    fn a_message_after_server_hello_done_is_refused_however_it_is_framed() {
        let pki = pki_p256();
        // Coalesced: the extra message is in the record the client is still
        // processing when it sends its flight.
        let err = refused(
            &pki,
            Script::new(Leaf::P256)
                .split(Split::Coalesced)
                .messages(|m| {
                    let cert = m[1].clone();
                    m.push(cert);
                }),
        );
        // Refused at the ServerHelloDone itself, before the client replies: the
        // flight is over, and the extra message is not part of it.
        assert!(
            matches!(
                err,
                ClientError::UnexpectedMessage {
                    expected: "nothing after ServerHelloDone",
                    got: HandshakeType::ServerHelloDone
                }
            ),
            "{err}"
        );
        // In its own record: a handshake record where a ChangeCipherSpec is due.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).messages(|m| {
                let cert = m[1].clone();
                m.push(cert);
            }),
        );
        assert_eq!(
            err,
            ClientError::UnexpectedContentType(ContentType::Handshake)
        );
    }

    #[test]
    fn a_server_hello_done_with_a_body_is_refused() {
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| f.hello_done_body = vec![0]),
        );
        assert_eq!(
            err,
            ClientError::Handshake(HandshakeError::Malformed("ServerHelloDone has a body"))
        );
    }

    #[test]
    fn a_certificate_request_is_answered_with_an_empty_certificate() {
        let pki = pki_p256();
        let (result, fake) = converse(
            &pki,
            Script::new(Leaf::P256).flight(|f, _| f.certificate_request = true),
        );
        assert!(result.is_ok(), "{:?}", result.err());
        assert_eq!(
            fake.client_types,
            vec![HandshakeType::Certificate, HandshakeType::ClientKeyExchange]
        );
        assert_eq!(
            fake.client_finished_ok,
            Some(true),
            "the empty Certificate is in the transcript"
        );

        // Without a request, no Certificate is sent at all.
        let (_, fake) = converse(&pki, Script::new(Leaf::P256));
        assert_eq!(fake.client_types, vec![HandshakeType::ClientKeyExchange]);
    }

    // ------------------------------------------------------------------ floods

    /// Warnings are ignored up to a limit, empty handshake records likewise:
    /// the peer pays nothing for either and this side pays a trip through the
    /// state machine, so a run of them ends the connection.
    #[test]
    fn floods_of_warnings_and_empty_records_are_cut_off_in_the_handshake() {
        use rusty_tls::handrolled::limits::{Flood, MAX_EMPTY_RECORDS, MAX_WARNING_ALERTS};

        let pki = pki_p256();
        let warning = vec![21u8, 3, 3, 0, 2, 1, 100];
        let empty = vec![22u8, 3, 3, 0, 0];
        let cases = [
            (warning, MAX_WARNING_ALERTS, Flood::WarningAlerts),
            (empty, MAX_EMPTY_RECORDS, Flood::EmptyRecords),
        ];
        for (record, limit, flood) in cases {
            let padded = |count: u32| {
                let record = record.clone();
                Script::new(Leaf::P256)
                    .records(move |r| (0..count).for_each(|_| r.insert(0, record.clone())))
            };
            let (result, _) = converse(&pki, padded(limit));
            assert!(
                result.is_ok(),
                "{flood:?}: a run within the limit is ignored"
            );
            assert_eq!(refused(&pki, padded(limit + 1)), ClientError::Flood(flood));
        }
    }

    // --------------------------------------------------------- ChangeCipherSpec

    #[test]
    fn a_change_cipher_spec_at_any_time_but_one_is_refused() {
        let pki = pki_p256();
        let ccs = vec![20u8, 3, 3, 0, 1, 1];
        // At every position in the server's first flight.
        for position in 0..4 {
            let ccs = ccs.clone();
            let err = refused(
                &pki,
                Script::new(Leaf::P256).records(move |r| r.insert(position, ccs.clone())),
            );
            assert_eq!(
                err,
                ClientError::UnexpectedChangeCipherSpec,
                "before record {position}"
            );
        }
        // Before anything at all.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).records(move |r| r.insert(0, ccs.clone())),
        );
        assert_eq!(err, ClientError::UnexpectedChangeCipherSpec);
    }

    #[test]
    fn a_malformed_change_cipher_spec_is_refused() {
        let pki = pki_p256();
        for mode in [Finish::CcsValueTwo, Finish::CcsEmpty, Finish::CcsTwoOctets] {
            let err = refused(&pki, Script::new(Leaf::P256).finish(mode));
            assert_eq!(err, ClientError::UnexpectedChangeCipherSpec, "{mode:?}");
        }
    }

    #[test]
    fn a_second_change_cipher_spec_is_refused() {
        let pki = pki_p256();
        let (result, _) = converse(&pki, Script::new(Leaf::P256).finish(Finish::CcsTwice));
        assert!(
            result.is_err(),
            "a duplicated ChangeCipherSpec must not be tolerated"
        );
    }

    // ------------------------------------------------------------- Finished

    #[test]
    fn a_finished_that_does_not_verify_is_refused() {
        let pki = pki_p256();
        assert_eq!(
            refused(
                &pki,
                Script::new(Leaf::P256).finish(Finish::WrongVerifyData)
            ),
            ClientError::BadFinished
        );
        assert_eq!(
            refused(
                &pki,
                Script::new(Leaf::P256).finish(Finish::ReflectedClientLabel)
            ),
            ClientError::BadFinished
        );
    }

    #[test]
    fn a_finished_under_the_wrong_key_or_unprotected_is_refused() {
        let pki = pki_p256();
        assert_eq!(
            refused(&pki, Script::new(Leaf::P256).finish(Finish::WrongKey)),
            ClientError::Record(RecordError::Decrypt)
        );
        let err = refused(&pki, Script::new(Leaf::P256).finish(Finish::Plaintext));
        assert!(matches!(err, ClientError::Record(_)), "{err}");
    }

    #[test]
    fn a_finished_without_a_change_cipher_spec_is_refused() {
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256).finish(Finish::NoChangeCipherSpec),
        );
        assert_eq!(
            err,
            ClientError::UnexpectedContentType(ContentType::Handshake)
        );
    }

    #[test]
    fn a_malformed_finished_is_refused() {
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256).finish(Finish::ShortVerifyData),
        );
        // A Finished of the wrong length does not verify (BoGo
        // `TrailingMessageData-ServerFinished`): decrypt_error, not decode_error.
        assert_eq!(err, ClientError::BadFinished);
    }

    #[test]
    fn a_message_after_the_finished_is_refused() {
        let pki = pki_p256();
        let err = refused(
            &pki,
            Script::new(Leaf::P256).finish(Finish::TrailingMessage),
        );
        assert_eq!(
            err,
            unexpected("nothing after Finished", HandshakeType::HelloRequest)
        );
    }

    // ---------------------------------------------------------------- alerts

    #[test]
    fn a_fatal_alert_ends_the_handshake_and_is_reported() {
        let pki = pki_p256();
        for description in [40u8, 70, 80, 112, 255] {
            let err = refused(
                &pki,
                Script::new(Leaf::P256).records(move |r| *r = vec![alert_record(2, description)]),
            );
            assert_eq!(
                err,
                peer_alert(description, AlertLevel::Fatal),
                "{description}"
            );
        }
    }

    #[test]
    fn a_warning_alert_is_advisory_but_close_notify_is_not() {
        let pki = pki_p256();
        // unrecognized_name (112) as a warning is how servers say "I ignored your SNI".
        let (result, _) = converse(
            &pki,
            Script::new(Leaf::P256).records(|r| r.insert(1, alert_record(1, 112))),
        );
        assert!(result.is_ok(), "{:?}", result.err());

        // close_notify cannot be orderly before the handshake is done.
        let err = refused(
            &pki,
            Script::new(Leaf::P256).records(|r| r.insert(1, alert_record(1, 0))),
        );
        assert_eq!(err, peer_alert(0, AlertLevel::Warning));
    }

    #[test]
    fn a_malformed_alert_is_refused() {
        let pki = pki_p256();
        for body in [vec![2u8], vec![2, 40, 0], vec![]] {
            let label = format!("{body:?}");
            let err = refused(
                &pki,
                Script::new(Leaf::P256).records(move |r| {
                    let mut record = vec![21, 3, 3, 0, body.len() as u8];
                    record.extend_from_slice(&body);
                    r.insert(0, record);
                }),
            );
            assert_eq!(
                err,
                ClientError::UnexpectedContentType(ContentType::Alert),
                "{label}"
            );
        }
    }

    #[test]
    fn a_protected_alert_in_place_of_the_finished_is_reported() {
        let pki = pki_p256();
        let err = refused(&pki, Script::new(Leaf::P256).finish(Finish::FatalAlert));
        assert_eq!(err, peer_alert(40, AlertLevel::Fatal));
        let err = refused(&pki, Script::new(Leaf::P256).finish(Finish::MalformedAlert));
        assert_eq!(err, ClientError::UnexpectedContentType(ContentType::Alert));
    }

    // --------------------------------------------------------------- framing

    /// A handshake whose only job is to be fed raw records.
    fn fresh(then: impl FnOnce(&mut ClientHandshake12<'_>)) {
        let anchors: [TrustAnchor<'_>; 0] = [];
        let cfg = ClientConfig12 {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: &[NamedGroup::X25519, NamedGroup::SecP256R1],
            cipher_suites: CipherSuite12::SUPPORTED,
        };
        let (mut client, _) = ClientHandshake12::start(&cfg).expect("starts");
        then(&mut client);
    }

    #[test]
    fn malformed_records_are_refused_before_anything_is_parsed() {
        fresh(|c| {
            assert_eq!(
                c.read_record(&[22, 3]),
                Err(ClientError::Record(RecordError::Truncated {
                    len: 2,
                    min: 5
                }))
            );
        });
        fresh(|c| {
            // The header says 10, the record holds 2.
            let err = c.read_record(&[22, 3, 3, 0, 10, 1, 2]);
            assert_eq!(
                err,
                Err(ClientError::Record(RecordError::LengthMismatch {
                    declared: 10,
                    available: 2
                }))
            );
        });
        fresh(|c| {
            // Longer than any TLS 1.2 record may be.
            let declared = 16_384 + 2048 + 1;
            let mut record = vec![22, 3, 3, (declared >> 8) as u8, declared as u8];
            record.extend(std::iter::repeat_n(0u8, declared));
            assert_eq!(
                c.read_record(&record),
                Err(ClientError::Record(RecordError::EncryptedFragmentTooLong {
                    len: declared
                }))
            );
        });
        fresh(|c| {
            // Not TLS at all: major version 2 (SSLv2) and 4.
            for major in [2u8, 4, 0, 0xff] {
                fresh(|c2| {
                    assert!(
                        matches!(
                            c2.read_record(&[22, major, 3, 0, 1, 0]),
                            Err(ClientError::Record(RecordError::UnexpectedVersion(_)))
                        ),
                        "{major}"
                    );
                });
            }
            let _ = c;
        });
    }

    #[test]
    fn a_record_type_that_has_no_place_is_refused() {
        // Application data and heartbeat before there are keys, and unassigned types.
        for typ in [23u8, 24, 25, 0, 255] {
            fresh(|c| {
                let err = c.read_record(&[typ, 3, 3, 0, 1, 0]);
                assert_eq!(
                    err,
                    Err(ClientError::UnexpectedContentType(ContentType::from_u8(
                        typ
                    ))),
                    "{typ}"
                );
            });
        }
    }

    #[test]
    fn a_handshake_message_larger_than_the_buffer_is_refused_before_it_arrives() {
        fresh(|c| {
            // A ServerHello header declaring 200,000 octets, then filler.
            let mut chunk = vec![2u8, 0x03, 0x0d, 0x40];
            chunk.resize(16_000, 0xaa);
            let mut result = Ok(Vec::new());
            for _ in 0..16 {
                let mut record = vec![22, 3, 3, (chunk.len() >> 8) as u8, chunk.len() as u8];
                record.extend_from_slice(&chunk);
                result = c.read_record(&record);
                if result.is_err() {
                    break;
                }
                chunk = vec![0xaa; 16_000];
            }
            assert_eq!(result, Err(ClientError::HandshakeTooLarge));
        });
    }

    #[test]
    fn a_failed_handshake_stays_failed_and_yields_no_connection() {
        let anchors: [TrustAnchor<'_>; 0] = [];
        let cfg = ClientConfig12 {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: &[NamedGroup::X25519, NamedGroup::SecP256R1],
            cipher_suites: CipherSuite12::SUPPORTED,
        };
        let (mut client, _) = ClientHandshake12::start(&cfg).expect("starts");
        assert!(client.read_record(&[21, 3, 3, 0, 2, 2, 40]).is_err());
        // Every later record, even a well-formed ServerHello header, is refused.
        assert_eq!(
            client.read_record(&[22, 3, 3, 0, 4, 2, 0, 0, 0]),
            Err(ClientError::Failed)
        );
        assert!(!client.is_finished());
        assert_eq!(client.into_connection().err(), Some(ClientError::Failed));
    }
}

// ------------------------------------------------- the established connection

mod connection {
    use super::*;
    use rusty_tls::handrolled::client::{Alert, AlertDescription, AlertLevel};
    use rusty_tls::handrolled::handshake::HandshakeType;
    use rusty_tls::handrolled::record::{ContentType, RecordError};

    fn established(leaf: Leaf) -> (Connection12, Fake) {
        let pki = pki(leaf, SERVER);
        let (result, fake) = converse(&pki, Script::new(leaf));
        (result.expect("handshake completes"), fake)
    }

    #[test]
    fn a_renegotiation_request_is_refused_and_the_connection_carries_on() {
        let (mut conn, mut fake) = established(Leaf::P256);
        let hello_request = fake.send_as(ContentType::Handshake, &[0, 0, 0, 0]);
        let reply = match conn.read(&hello_request).expect("handled") {
            Incoming12::Reply(bytes) => bytes,
            other => panic!("expected a refusal to send, got {other:?}"),
        };
        // The refusal is a protected warning alert, no_renegotiation (100).
        assert_eq!(fake.open(&reply), (ContentType::Alert, vec![1, 100]));
        // And the connection is still usable in both directions.
        assert_eq!(
            conn.read(&fake.send(b"still here")).expect("reads"),
            Incoming12::Application(b"still here".to_vec())
        );
        let written = conn.write(b"and here").expect("writes");
        assert_eq!(fake.open(&written).1, b"and here");
    }

    #[test]
    fn any_other_post_handshake_handshake_message_is_fatal() {
        // NewSessionTicket, KeyUpdate (a TLS 1.3 message), ClientHello, Finished.
        for typ in [4u8, 24, 1, 20] {
            let (mut conn, mut fake) = established(Leaf::P256);
            let record = fake.send_as(ContentType::Handshake, &[typ, 0, 0, 0]);
            let err = conn.read(&record).expect_err("refused");
            assert!(
                matches!(err, ClientError::UnexpectedMessage { .. }),
                "type {typ}: {err}"
            );
            // Permanently: even good data is refused afterwards.
            assert_eq!(
                conn.read(&fake.send(b"x")).err(),
                Some(ClientError::Failed),
                "type {typ}"
            );
            assert!(conn.write(b"x").is_err());
        }
    }

    #[test]
    fn a_record_that_fails_to_authenticate_ends_the_connection_for_good() {
        let (mut conn, mut fake) = established(Leaf::P256);
        let mut record = fake.send(b"genuine");
        let last = record.len() - 1;
        record[last] ^= 1;
        assert_eq!(
            conn.read(&record).err(),
            Some(ClientError::Record(RecordError::Decrypt))
        );
        // The next, perfectly good, record is refused: a stream that has had a
        // forged record in it cannot be trusted to continue.
        assert_eq!(
            conn.read(&fake.send(b"after")).err(),
            Some(ClientError::Failed)
        );
        assert!(conn.write(b"x").is_err(), "writes stop too");
        assert!(conn.close().is_err());
    }

    #[test]
    fn replayed_and_reordered_records_are_refused() {
        let (mut conn, mut fake) = established(Leaf::P256);
        let first = fake.send(b"one");
        let second = fake.send(b"two");
        assert_eq!(
            conn.read(&first).expect("first"),
            Incoming12::Application(b"one".to_vec())
        );
        // The first again: its sequence number has been used.
        assert_eq!(
            conn.read(&first).err(),
            Some(ClientError::Record(RecordError::Decrypt))
        );

        let (mut conn, mut fake) = established(Leaf::P256);
        let first = fake.send(b"one");
        let second_record = fake.send(b"two");
        let _ = first;
        // The second arriving first: out of order.
        assert_eq!(
            conn.read(&second_record).err(),
            Some(ClientError::Record(RecordError::Decrypt))
        );
        let _ = second;
    }

    #[test]
    fn alerts_after_the_handshake() {
        // A fatal alert is reported.
        let (mut conn, mut fake) = established(Leaf::P256);
        let record = fake.send_as(ContentType::Alert, &[2, 80]);
        assert_eq!(
            conn.read(&record).err(),
            Some(ClientError::PeerAlert(Alert {
                level: AlertLevel::Fatal,
                description: AlertDescription(80)
            }))
        );

        // A warning is advisory and the connection carries on.
        let (mut conn, mut fake) = established(Leaf::P256);
        assert_eq!(
            conn.read(&fake.send_as(ContentType::Alert, &[1, 112]))
                .expect("handled"),
            Incoming12::Handled
        );
        assert_eq!(
            conn.read(&fake.send(b"ok")).expect("reads"),
            Incoming12::Application(b"ok".to_vec())
        );

        // close_notify is an orderly close, at either alert level.
        for level in [1u8, 2] {
            let (mut conn, mut fake) = established(Leaf::P256);
            assert_eq!(
                conn.read(&fake.send_as(ContentType::Alert, &[level, 0]))
                    .expect("closes"),
                Incoming12::Closed
            );
            assert!(
                conn.write(b"too late").is_err(),
                "no writes after the peer closed"
            );
        }

        // A malformed alert is refused.
        for body in [vec![2u8], vec![2, 40, 0], vec![]] {
            let (mut conn, mut fake) = established(Leaf::P256);
            assert_eq!(
                conn.read(&fake.send_as(ContentType::Alert, &body)).err(),
                Some(ClientError::UnexpectedContentType(ContentType::Alert)),
                "{body:?}"
            );
        }
    }

    #[test]
    fn a_record_type_that_has_no_place_after_the_handshake_is_refused() {
        // A second ChangeCipherSpec, a heartbeat, and an unassigned type.
        for typ in [
            ContentType::ChangeCipherSpec,
            ContentType::Unknown(24),
            ContentType::Unknown(99),
        ] {
            let (mut conn, mut fake) = established(Leaf::P256);
            let err = conn.read(&fake.send_as(typ, &[1])).expect_err("refused");
            assert_eq!(err, ClientError::UnexpectedContentType(typ));
        }
    }

    #[test]
    fn an_empty_application_record_is_not_an_error() {
        // RFC 5246 allows zero-length application data (some stacks send one as
        // a countermeasure). It surfaces as empty data rather than as an error
        // or as nothing, so a caller can tell it happened.
        let (mut conn, mut fake) = established(Leaf::P256);
        assert_eq!(
            conn.read(&fake.send(b"")).expect("reads"),
            Incoming12::Application(Vec::new())
        );
    }

    #[test]
    fn writing_nothing_sends_nothing_and_large_writes_are_split() {
        let (mut conn, mut fake) = established(Leaf::P256);
        assert!(conn.write(b"").expect("writes").is_empty());

        // 40,000 octets is three records of at most 2^14.
        let data: Vec<u8> = (0..40_000u32).map(|i| (i % 253) as u8).collect();
        let mut stream = conn.write(&data).expect("writes");
        let mut got = Vec::new();
        let mut records = 0;
        for record in take_records(&mut stream) {
            let (typ, fragment) = fake.open(&record);
            assert_eq!(typ, ContentType::ApplicationData);
            assert!(fragment.len() <= 16_384);
            got.extend_from_slice(&fragment);
            records += 1;
        }
        assert_eq!(records, 3);
        assert_eq!(got, data);
    }

    #[test]
    fn close_sends_a_protected_close_notify_and_stops_writes() {
        let (mut conn, mut fake) = established(Leaf::P256);
        let alert = conn.close().expect("close");
        assert_eq!(fake.open(&alert), (ContentType::Alert, vec![1, 0]));
        assert!(conn.write(b"after").is_err());
        // The peer's own close_notify is still read.
        assert_eq!(
            conn.read(&fake.send_as(ContentType::Alert, &[1, 0]))
                .expect("reads"),
            Incoming12::Closed
        );
    }

    #[test]
    fn the_connection_prints_no_key_material() {
        let (conn, _) = established(Leaf::P256);
        let printed = format!("{conn:?}");
        assert!(printed.starts_with("Connection12"), "{printed}");
        assert!(
            !printed.contains("key") && !printed.contains("secret"),
            "{printed}"
        );
    }

    #[test]
    fn peer_certificates_are_the_chain_the_server_sent() {
        let pki = pki(Leaf::P256, SERVER);
        let (result, fake) = converse(&pki, Script::new(Leaf::P256));
        let conn = result.expect("completes");
        assert_eq!(conn.peer_certificates(), &[fake.leaf_der().to_vec()]);
        let _ = HandshakeType::Finished;
    }
}

// ------------------------------------------------------------ hostile input

/// A tiny deterministic generator: reproducible failures matter more than
/// unpredictability, and nothing here protects anything.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// Nothing a peer sends makes the client panic, and random bytes never complete
/// a handshake. A panic here is a denial of service reachable by anybody.
#[test]
fn random_records_never_panic_a_handshake_and_never_complete_one() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let anchors: [TrustAnchor<'_>; 0] = [];
    let cfg = ClientConfig12 {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[
            NamedGroup::X25519,
            NamedGroup::SecP256R1,
            NamedGroup::SecP384R1,
        ],
        cipher_suites: CipherSuite12::SUPPORTED,
    };
    for round in 0..3000 {
        let (mut client, _) = ClientHandshake12::start(&cfg).expect("starts");
        // Several records per round, with plausible headers so the content, not
        // the framing, is what gets parsed.
        for _ in 0..(1 + rng.next() % 4) {
            let len = (rng.next() % 300) as usize;
            let mut record = vec![0u8; 5];
            record[0] = [20u8, 21, 22, 22, 22, 23][(rng.next() % 6) as usize];
            record[1] = 3;
            record[2] = 3;
            record[3..5].copy_from_slice(&(len as u16).to_be_bytes());
            record.extend(rng.bytes(len));
            let _ = client.read_record(&record);
        }
        assert!(
            !client.is_finished(),
            "round {round}: random bytes completed a handshake"
        );
    }
}

#[test]
fn random_records_never_panic_an_established_connection_and_never_authenticate() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..200 {
        let pki = pki(Leaf::P256, SERVER);
        let (result, _) = converse(&pki, Script::new(Leaf::P256));
        let mut conn = result.expect("handshake");
        for _ in 0..20 {
            let len = (rng.next() % 200) as usize;
            let mut record = vec![[21u8, 22, 23][(rng.next() % 3) as usize], 3, 3];
            record.extend_from_slice(&(len as u16).to_be_bytes());
            record.extend(rng.bytes(len));
            assert!(
                !matches!(conn.read(&record), Ok(Incoming12::Application(_))),
                "a forged record surfaced as application data"
            );
        }
    }
}

/// Flip one bit in each byte of everything a valid server sends. The only
/// flips that may leave the handshake completing are the ones the protocol says
/// are ignored: the minor version byte of an *unprotected* record header
/// (RFC 5246 appendix E). Everything else (any content, any length, any
/// protected header byte) must break the handshake.
///
/// This is the strongest single statement the suite makes: it is a claim about
/// every byte, not about the bytes somebody thought to corrupt.
#[test]
fn only_the_ignored_header_bytes_can_change_without_breaking_the_handshake() {
    let pki = pki_p256();

    // Learn the shape of a correct transmission.
    let mut first = Vec::new();
    let mut last = Vec::new();
    let (ok, _) = converse(
        &pki,
        Script::new(Leaf::P256)
            .records(|r| first = r.iter().map(Vec::len).collect())
            .final_records(|r| last = r.iter().map(Vec::len).collect()),
    );
    assert!(ok.is_ok());
    assert_eq!(last.len(), 2, "ChangeCipherSpec and Finished");

    let mut completed = 0usize;
    let mut tried = 0usize;
    let mut check = |in_first: bool, record: usize, byte: usize, len: usize| {
        let bit = 1u8 << (byte % 8);
        // ECDSA signatures vary in length from run to run, so a record can
        // be a byte shorter than the shape measured above. A flip that finds
        // no byte to flip is skipped, not counted: it would leave the
        // handshake intact and be mistaken for an undetected change.
        let applied = std::cell::Cell::new(false);
        let flip = |records: &mut Vec<Vec<u8>>| {
            if let Some(b) = records.get_mut(record).and_then(|r| r.get_mut(byte)) {
                *b ^= bit;
                applied.set(true);
            }
        };
        let script = if in_first {
            Script::new(Leaf::P256).records(flip)
        } else {
            Script::new(Leaf::P256).final_records(flip)
        };
        let (result, _) = converse(&pki, script);
        if !applied.get() {
            return;
        }
        tried += 1;

        // Which bytes are exempt: byte 2 (minor version) of a plaintext record,
        // which is every record of the first flight and the ChangeCipherSpec.
        let plaintext = in_first || record == 0;
        let exempt = plaintext && byte == 2;
        if result.is_ok() {
            completed += 1;
            assert!(
                exempt,
                "flipping bit {} of byte {byte} (record {record}, {}) of {len} went unnoticed",
                byte % 8,
                if in_first {
                    "first flight"
                } else {
                    "final flight"
                }
            );
        }
    };
    for (record, len) in first.iter().enumerate() {
        for byte in 0..*len {
            check(true, record, byte, *len);
        }
    }
    for (record, len) in last.iter().enumerate() {
        for byte in 0..*len {
            check(false, record, byte, *len);
        }
    }

    // Guard against the sweep being vacuous: it ran, and the exempt bytes (one
    // per plaintext record) are exactly the ones that completed.
    assert!(tried > 400, "the sweep only tried {tried} flips");
    assert_eq!(
        completed,
        first.len() + 1,
        "one exempt byte per plaintext record"
    );
}
