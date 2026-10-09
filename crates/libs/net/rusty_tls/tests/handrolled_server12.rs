//! The TLS 1.2 server handshake — stage 4b-iv.
//!
//! # Evidence, in the order it was gathered
//!
//! 1. **A live rustls client** (this file, `interop`): every suite, group and
//!    key type, data both ways, `close_notify`, client authentication. rustls
//!    has not read this implementation, so a completed handshake is evidence
//!    about TLS 1.2 and not about internal consistency.
//! 2. **OpenSSL `s_client` over a socket**
//!    (`handrolled_server12_socket_interop.rs`), run in CI.
//! 3. **This crate's own client** (`self_interop`): each end is checked
//!    against the other, which proves only that they agree — it is here for
//!    what the live peers cannot do, such as an exhaustive refusal sweep.
//! 4. **A scripted client** (`refusals`) that is deliberately wrong in one
//!    way at a time, each required to be refused with a named error and the
//!    right alert, and each paired with a control that is right.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::ClientError;
use rusty_tls::handrolled::client::{record_length, AlertDescription};
use rusty_tls::handrolled::client12::{
    CipherSuite12, ClientConfig12, ClientHandshake12, Connection12, Incoming12,
};
use rusty_tls::handrolled::handshake::{extension, messages, HandshakeError, HandshakeType};
use rusty_tls::handrolled::handshake12::{
    encode_client_key_exchange, message, Certificate12, ClientHello12, ServerHello12,
    ServerKeyExchange,
};
use rusty_tls::handrolled::kx::KeyExchange;
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::record::ContentType;
use rusty_tls::handrolled::record12::{Opener, Sealer};
use rusty_tls::handrolled::schedule::Hash;
use rusty_tls::handrolled::schedule12::{
    extended_master_secret, finished_verify_data, key_block, verify_finished, MasterSecret, Side,
};
use rusty_tls::handrolled::server::{ClientAuth, ServerError};
use rusty_tls::handrolled::server12::{ServerConfig12, ServerHandshake12};
use rusty_tls::handrolled::sign::SigningKey;
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

fn signing_key(pki: &Pki, leaf: Leaf) -> SigningKey {
    match leaf {
        Leaf::P256 => SigningKey::ecdsa_p256(&pki.leaf_pkcs8),
        Leaf::P384 => SigningKey::ecdsa_p384(&pki.leaf_pkcs8),
        Leaf::Ed25519 => SigningKey::ed25519(&pki.leaf_pkcs8),
        Leaf::Rsa => SigningKey::rsa(&pki.leaf_pkcs8),
    }
    .expect("the leaf key loads")
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

const ALL_GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

/// Everything a server config borrows, owned in one place.
struct Server {
    pki: Pki,
    certificates: Vec<Vec<u8>>,
    key: SigningKey,
}

impl Server {
    fn new(leaf: Leaf) -> Self {
        let pki = pki(leaf, SERVER);
        let certificates = vec![pki.leaf_der.clone(), pki.root_der.clone()];
        let key = signing_key(&pki, leaf);
        Self {
            pki,
            certificates,
            key,
        }
    }

    fn config<'a>(
        &'a self,
        suites: &'a [CipherSuite12],
        groups: &'a [NamedGroup],
        client_auth: Option<&'a ClientAuth<'a>>,
    ) -> ServerConfig12<'a> {
        ServerConfig12 {
            certificates: &self.certificates,
            key: &self.key,
            cipher_suites: suites,
            groups,
            client_auth,
        }
    }
}

// ---------------------------------------------------------------------------
// A real rustls client, restricted to TLS 1.2
// ---------------------------------------------------------------------------

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

fn roots(root_der: &[u8]) -> rustls::RootCertStore {
    let mut store = rustls::RootCertStore::empty();
    store
        .add(CertificateDer::from(root_der.to_vec()))
        .expect("rustls accepts the root");
    store
}

fn rustls_client_with(
    root_der: &[u8],
    provider: Arc<rustls::crypto::CryptoProvider>,
    versions: &[&'static rustls::SupportedProtocolVersion],
    identity: Option<&Pki>,
) -> rustls::ClientConnection {
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .expect("versions")
        .with_root_certificates(roots(root_der));
    let config = match identity {
        None => builder.with_no_client_auth(),
        Some(client) => builder
            .with_client_auth_cert(client.chain.clone(), client.key.clone_key())
            .expect("client auth config"),
    };
    rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from(SERVER).expect("name"),
    )
    .expect("client connection")
}

fn rustls_client_12(root_der: &[u8]) -> rustls::ClientConnection {
    rustls_client_with(
        root_der,
        Arc::new(rustls::crypto::ring::default_provider()),
        &[&rustls::version::TLS12],
        None,
    )
}

/// Feed bytes to a rustls client and collect what it wants to send, tolerating
/// a client that gives up (its alert is still queued).
fn pump_client(client: &mut rustls::ClientConnection, input: &[u8]) -> (Vec<u8>, Option<String>) {
    let mut refusal = None;
    if !input.is_empty() {
        let mut cursor = std::io::Cursor::new(input);
        while client.read_tls(&mut cursor).unwrap_or(0) > 0 {
            if let Err(err) = client.process_new_packets() {
                refusal = Some(err.to_string());
                break;
            }
        }
    }
    if refusal.is_none() {
        if let Err(err) = client.process_new_packets() {
            refusal = Some(err.to_string());
        }
    }
    let mut out = Vec::new();
    while client.wants_write() {
        client.write_tls(&mut out).expect("write_tls");
    }
    (out, refusal)
}

#[derive(Debug)]
struct Established {
    connection: Connection12,
    client: rustls::ClientConnection,
}

/// What a handshake against rustls produced: the server's verdict, and every
/// alert the server asked to be sent.
struct Outcome {
    result: Result<Established, ServerError>,
    alert: Option<Vec<u8>>,
    client_refusal: Option<String>,
}

/// Run a handshake to completion between `server` and a rustls client.
fn drive(config: &ServerConfig12<'_>, mut client: rustls::ClientConnection) -> Outcome {
    let mut server = ServerHandshake12::new(config).expect("a usable config");
    let mut to_server = pump_client(&mut client, &[]).0;
    let mut client_refusal = None;

    for _ in 0..16 {
        let mut to_client = Vec::new();
        for record in take_records(&mut to_server) {
            match server.read_record(&record) {
                Ok(reply) => to_client.extend_from_slice(&reply),
                Err(error) => {
                    let alert = server.alert_record(&error);
                    return Outcome {
                        result: Err(error),
                        alert,
                        client_refusal,
                    };
                }
            }
        }
        if server.is_finished() {
            // The server's last flight is still to be delivered to the client.
            let (_, refusal) = pump_client(&mut client, &to_client);
            let connection = server.into_connection().expect("finished");
            return Outcome {
                result: Ok(Established { connection, client }),
                alert: None,
                client_refusal: refusal,
            };
        }
        let (bytes, refusal) = pump_client(&mut client, &to_client);
        client_refusal = refusal.or(client_refusal);
        if bytes.is_empty() {
            break;
        }
        to_server = bytes;
    }
    panic!("the handshake neither finished nor failed (client said {client_refusal:?})");
}

fn handshake(server: &Server) -> Established {
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let outcome = drive(&config, rustls_client_12(&server.pki.root_der));
    assert_eq!(outcome.client_refusal, None);
    outcome.result.expect("the handshake completes")
}

/// Client to server: rustls writes, this connection reads.
fn client_to_server(established: &mut Established, data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut got = Vec::new();
    for chunk in data.chunks(16 * 1024) {
        established
            .client
            .writer()
            .write_all(chunk)
            .expect("client write");
        let (bytes, _) = pump_client(&mut established.client, &[]);
        let mut stream = bytes;
        for record in take_records(&mut stream) {
            match established.connection.read(&record).expect("server reads") {
                Incoming12::Application(plain) => got.extend_from_slice(&plain),
                other => panic!("expected application data, got {other:?}"),
            }
        }
    }
    got
}

/// Server to client: this connection writes, rustls reads.
fn server_to_client(established: &mut Established, data: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut stream = established.connection.write(data).expect("write");
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    for record in take_records(&mut stream) {
        let (_, refusal) = pump_client(&mut established.client, &record);
        assert_eq!(refusal, None, "rustls refused our application data");
        while let Ok(n) = established.client.reader().read(&mut buf) {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
    }
    got
}

// ---------------------------------------------------------------------------
// Interop — the tests that carry this file
// ---------------------------------------------------------------------------

#[test]
fn a_full_tls12_handshake_with_a_rustls_client_completes_and_carries_data() {
    for leaf in [Leaf::P256, Leaf::P384, Leaf::Ed25519, Leaf::Rsa] {
        let server = Server::new(leaf);
        let mut established = handshake(&server);

        let request = b"GET / HTTP/1.1\r\nhost: tls12.example\r\n\r\n";
        assert_eq!(
            client_to_server(&mut established, request),
            request,
            "{leaf:?}"
        );
        let response = vec![0x5a; 40_000];
        assert_eq!(
            server_to_client(&mut established, &response),
            response,
            "{leaf:?}"
        );
        assert!(
            established.connection.peer_certificates().is_empty(),
            "no client certificate was asked for"
        );
    }
}

/// Each suite and group is the *only* one rustls offers, so each is actually
/// negotiated rather than merely available. The key type decides which suites
/// are possible; the rest must be refused by the server, not by luck.
#[test]
fn every_suite_and_group_completes_with_a_rustls_client() {
    use rustls::crypto::ring::{cipher_suite as cs, kx_group};
    let suites: [(rustls::SupportedCipherSuite, CipherSuite12, Leaf); 6] = [
        (
            cs::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            CipherSuite12::ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            Leaf::P256,
        ),
        (
            cs::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            CipherSuite12::ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            Leaf::P384,
        ),
        (
            cs::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
            CipherSuite12::ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
            Leaf::Ed25519,
        ),
        (
            cs::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            CipherSuite12::ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            Leaf::Rsa,
        ),
        (
            cs::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            CipherSuite12::ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            Leaf::Rsa,
        ),
        (
            cs::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
            CipherSuite12::ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
            Leaf::Rsa,
        ),
    ];
    let groups: [(&'static dyn rustls::crypto::SupportedKxGroup, NamedGroup); 3] = [
        (kx_group::X25519, NamedGroup::X25519),
        (kx_group::SECP256R1, NamedGroup::SecP256R1),
        (kx_group::SECP384R1, NamedGroup::SecP384R1),
    ];

    for (rustls_suite, suite, leaf) in suites {
        let server = Server::new(leaf);
        for (rustls_group, group) in groups {
            let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
            let client = rustls_client_with(
                &server.pki.root_der,
                restricted_provider(Some(rustls_suite), Some(rustls_group)),
                &[&rustls::version::TLS12],
                None,
            );
            let outcome = drive(&config, client);

            // RFC 8422 §5.1: a certificate whose curve the client did not list
            // cannot be used, and a client offering one group lists only that.
            let curve = match leaf {
                Leaf::P256 => Some(NamedGroup::SecP256R1),
                Leaf::P384 => Some(NamedGroup::SecP384R1),
                _ => None,
            };
            if curve.is_some_and(|curve| curve != group) {
                let error = outcome.result.expect_err("the key's curve is not offered");
                assert_eq!(
                    error,
                    ServerError::NoSharedSignatureScheme,
                    "{leaf:?} {group:?}"
                );
                continue;
            }
            let mut established = outcome
                .result
                .unwrap_or_else(|e| panic!("{suite:?} {group:?} {leaf:?}: {e}"));
            assert_eq!(established.connection.suite(), suite);
            assert_eq!(client_to_server(&mut established, b"ping"), b"ping");
            assert_eq!(server_to_client(&mut established, b"pong"), b"pong");
        }
    }
}

#[test]
fn close_notify_works_in_both_directions() {
    let server = Server::new(Leaf::P256);
    let mut established = handshake(&server);

    let closing = established.connection.close().expect("close");
    let (_, refusal) = pump_client(&mut established.client, &closing);
    assert_eq!(refusal, None);

    established.client.send_close_notify();
    let (bytes, _) = pump_client(&mut established.client, &[]);
    let mut stream = bytes;
    let records = take_records(&mut stream);
    assert_eq!(records.len(), 1);
    // The server side already closed its writing half; reading is still open.
    assert_eq!(
        established.connection.read(&records[0]).expect("read"),
        Incoming12::Closed
    );
}

#[test]
fn a_tls13_only_client_is_refused_with_protocol_version() {
    let server = Server::new(Leaf::P256);
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let client = rustls_client_with(
        &server.pki.root_der,
        Arc::new(rustls::crypto::ring::default_provider()),
        &[&rustls::version::TLS13],
        None,
    );
    let outcome = drive(&config, client);
    let error = outcome.result.expect_err("a TLS 1.3 client is refused");
    assert!(matches!(error, ServerError::NotTls12(_)), "{error:?}");
    assert_eq!(error.alert(), Some(AlertDescription::PROTOCOL_VERSION));
    assert!(outcome.alert.is_some());
}

// ---------------------------------------------------------------------------
// Client authentication
// ---------------------------------------------------------------------------

fn client_pki_signed_by(server: &Pki) -> Pki {
    // The server's PKI signs only its own leaf, so a client identity needs a
    // chain to a root the *server* trusts; this PKI is that root's twin.
    let _ = server;
    pki(Leaf::P256, "client.example")
}

fn auth_anchors(roots: &[&Pki]) -> Vec<Vec<u8>> {
    roots.iter().map(|p| p.root_der.clone()).collect()
}

fn with_client_auth<R>(
    server: &Server,
    trusted: &Pki,
    required: bool,
    identity: Option<&Pki>,
    check: impl FnOnce(Outcome) -> R,
) -> R {
    let root_ders = auth_anchors(&[trusted]);
    let anchors: Vec<TrustAnchor<'_>> = root_ders.iter().map(|d| anchor(d)).collect();
    let auth = ClientAuth {
        anchors: &anchors,
        path: options(),
        required,
    };
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, Some(&auth));
    let client = rustls_client_with(
        &server.pki.root_der,
        Arc::new(rustls::crypto::ring::default_provider()),
        &[&rustls::version::TLS12],
        identity,
    );
    check(drive(&config, client))
}

#[test]
fn a_client_certificate_is_requested_verified_and_reported() {
    let server = Server::new(Leaf::P256);
    let client = client_pki_signed_by(&server.pki);
    with_client_auth(&server, &client, true, Some(&client), |outcome| {
        let mut established = outcome.result.expect("client auth completes");
        assert_eq!(
            established.connection.peer_certificates().first(),
            Some(&client.leaf_der)
        );
        assert_eq!(client_to_server(&mut established, b"hi"), b"hi");
    });
}

#[test]
fn every_client_key_type_authenticates() {
    for leaf in [Leaf::P256, Leaf::P384, Leaf::Ed25519, Leaf::Rsa] {
        let server = Server::new(Leaf::P256);
        let client = pki(leaf, "client.example");
        with_client_auth(&server, &client, true, Some(&client), |outcome| {
            let established = outcome
                .result
                .unwrap_or_else(|e| panic!("{leaf:?} client: {e}"));
            assert_eq!(established.connection.peer_certificates().len(), 2);
        });
    }
}

#[test]
fn a_missing_client_certificate_is_refused_when_required_and_tolerated_when_not() {
    let server = Server::new(Leaf::P256);
    let client = client_pki_signed_by(&server.pki);

    with_client_auth(&server, &client, true, None, |outcome| {
        let error = outcome.result.expect_err("required means required");
        assert_eq!(error, ServerError::ClientCertificateRequired);
        assert_eq!(error.alert(), Some(AlertDescription::CERTIFICATE_REQUIRED));
    });
    with_client_auth(&server, &client, false, None, |outcome| {
        let established = outcome.result.expect("optional means optional");
        assert!(established.connection.peer_certificates().is_empty());
    });
}

#[test]
fn a_client_certificate_from_an_unrelated_ca_is_refused() {
    let server = Server::new(Leaf::P256);
    let trusted = client_pki_signed_by(&server.pki);
    let stranger = pki(Leaf::P256, "client.example");
    with_client_auth(&server, &trusted, true, Some(&stranger), |outcome| {
        let error = outcome.result.expect_err("an unknown CA is refused");
        assert!(
            matches!(error, ServerError::ClientCertificate(_)),
            "{error:?}"
        );
        assert_eq!(error.alert(), Some(AlertDescription::BAD_CERTIFICATE));
    });
}

// ---------------------------------------------------------------------------
// This crate's own client: both ends of a handshake, in memory
// ---------------------------------------------------------------------------

fn client_config<'a>(
    anchors: &'a [TrustAnchor<'a>],
    suites: &'a [CipherSuite12],
) -> ClientConfig12<'a> {
    ClientConfig12 {
        server_name: ServerName::Dns(SERVER),
        anchors,
        path: options(),
        groups: ALL_GROUPS,
        cipher_suites: suites,
        identity: None,
    }
}

/// A frame around a handshake in flight, after the server has accepted the
/// ClientHello: the server, the client waiting for the server's Finished, and
/// the client's flight as separate records (`[handshake, ChangeCipherSpec,
/// Finished]`).
struct AtClientFlight<'a> {
    server: ServerHandshake12<'a>,
    client: ClientHandshake12<'a>,
    flight: Vec<Vec<u8>>,
}

fn at_client_flight<'a>(
    server_config: &'a ServerConfig12<'a>,
    client_config: &'a ClientConfig12<'a>,
) -> AtClientFlight<'a> {
    let (mut client, hello) = ClientHandshake12::start(client_config).expect("client starts");
    let mut server = ServerHandshake12::new(server_config).expect("server");
    let mut stream = server
        .read_record(&hello)
        .expect("the server accepts the hello");
    let mut reply = Vec::new();
    for record in take_records(&mut stream) {
        reply.extend(
            client
                .read_record(&record)
                .expect("the client accepts the flight"),
        );
    }
    let flight = take_records(&mut reply);
    assert_eq!(flight.len(), 3, "handshake, ChangeCipherSpec, Finished");
    AtClientFlight {
        server,
        client,
        flight,
    }
}

/// Run the rest of the handshake and return both connections.
fn finish(mut at: AtClientFlight<'_>) -> (Connection12, Connection12) {
    let mut reply = Vec::new();
    for record in &at.flight {
        reply.extend(at.server.read_record(record).expect("the server accepts"));
    }
    for record in take_records(&mut reply) {
        at.client.read_record(&record).expect("the client accepts");
    }
    assert!(at.server.is_finished() && at.client.is_finished());
    (
        at.client.into_connection().expect("client connection"),
        at.server.into_connection().expect("server connection"),
    )
}

fn with_configs<R>(
    server: &Server,
    suites: &[CipherSuite12],
    check: impl FnOnce(&ServerConfig12<'_>, &ClientConfig12<'_>) -> R,
) -> R {
    let anchors = [anchor(&server.pki.root_der)];
    let server_config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let client_config = client_config(&anchors, suites);
    check(&server_config, &client_config)
}

mod self_interop {
    use super::*;

    /// Every suite, with each key type that can authenticate it, and data both
    /// ways. Agreement with each other proves only consistency; the live peers
    /// above are what prove correctness.
    #[test]
    fn every_suite_and_key_type_completes_against_this_crates_client() {
        for leaf in [Leaf::P256, Leaf::P384, Leaf::Ed25519, Leaf::Rsa] {
            let server = Server::new(leaf);
            for suite in CipherSuite12::SUPPORTED {
                let (_, _, auth) = suite.parts().expect("supported");
                let fits = matches!(
                    (leaf, auth),
                    (
                        Leaf::Rsa,
                        rusty_tls::handrolled::client12::Authentication::Rsa
                    ) | (
                        Leaf::P256 | Leaf::P384 | Leaf::Ed25519,
                        rusty_tls::handrolled::client12::Authentication::Ecdsa
                    )
                );
                if !fits {
                    continue;
                }
                let suites = [*suite];
                with_configs(&server, &suites, |sc, cc| {
                    // The client offers a P-256 group too, for the ECDSA curves.
                    let (mut client, mut server) = finish(at_client_flight(sc, cc));
                    assert_eq!(client.suite(), *suite);
                    assert_eq!(server.suite(), *suite);

                    let mut wire = client.write(b"request").expect("write");
                    for record in take_records(&mut wire) {
                        assert_eq!(
                            server.read(&record).expect("read"),
                            Incoming12::Application(b"request".to_vec())
                        );
                    }
                    let mut wire = server.write(&vec![7u8; 50_000]).expect("write");
                    let mut got = 0;
                    for record in take_records(&mut wire) {
                        match client.read(&record).expect("read") {
                            Incoming12::Application(data) => got += data.len(),
                            other => panic!("{other:?}"),
                        }
                    }
                    assert_eq!(got, 50_000);
                });
            }
        }
    }

    /// The same bytes, delivered however the transport likes: the server must
    /// reassemble a ClientHello that arrives in pieces.
    #[test]
    fn a_client_hello_is_reassembled_however_it_is_framed() {
        let server = Server::new(Leaf::P256);
        with_configs(&server, CipherSuite12::SUPPORTED, |sc, cc| {
            let (_, hello) = ClientHandshake12::start(cc).expect("start");
            let fragment = &hello[5..];
            for size in [1usize, 2, 3, 5, 7, 50, fragment.len() - 1] {
                let mut srv = ServerHandshake12::new(sc).expect("server");
                let mut reply = Vec::new();
                for piece in fragment.chunks(size) {
                    let mut record = vec![22, 3, 3];
                    record.extend_from_slice(&(piece.len() as u16).to_be_bytes());
                    record.extend_from_slice(piece);
                    reply.extend(srv.read_record(&record).expect("a piece is accepted"));
                }
                assert!(!reply.is_empty(), "framing {size}: no flight");
            }
        });
    }

    #[test]
    fn close_notify_crosses_in_both_directions() {
        let server = Server::new(Leaf::P256);
        with_configs(&server, CipherSuite12::SUPPORTED, |sc, cc| {
            let (mut client, mut server) = finish(at_client_flight(sc, cc));
            let mut closing = server.close().expect("close");
            assert_eq!(
                client.read(&take_records(&mut closing)[0]).expect("read"),
                Incoming12::Closed
            );
            let mut closing = client.close().expect("close");
            assert_eq!(
                server.read(&take_records(&mut closing)[0]).expect("read"),
                Incoming12::Closed
            );
            assert!(matches!(server.write(b"late"), Err(ClientError::Failed)));
        });
    }
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// A ClientHello in a form that can be edited and put back.
#[derive(Clone)]
struct Hello {
    version: u16,
    random: Vec<u8>,
    session_id: Vec<u8>,
    suites: Vec<u16>,
    compression: Vec<u8>,
    extensions: Vec<(u16, Vec<u8>)>,
}

impl Hello {
    /// The ClientHello this crate's client sends, taken apart.
    fn honest() -> Self {
        let server = Server::new(Leaf::P256);
        with_configs(&server, CipherSuite12::SUPPORTED, |_, cc| {
            let (_, record) = ClientHandshake12::start(cc).expect("start");
            let parsed = messages(&record[5..]).expect("messages");
            let hello = ClientHello12::parse(parsed[0].body).expect("parses");
            Self {
                version: hello.version,
                random: hello.random.to_vec(),
                session_id: hello.session_id.to_vec(),
                suites: hello.cipher_suites.clone(),
                compression: hello.compression.to_vec(),
                extensions: hello
                    .extensions
                    .iter()
                    .map(|e| (e.typ, e.data.to_vec()))
                    .collect(),
            }
        })
    }

    fn without(mut self, typ: u16) -> Self {
        self.extensions.retain(|(t, _)| *t != typ);
        self
    }

    fn with(mut self, typ: u16, data: &[u8]) -> Self {
        self.extensions.retain(|(t, _)| *t != typ);
        self.extensions.push((typ, data.to_vec()));
        self
    }

    fn suites(mut self, suites: &[u16]) -> Self {
        self.suites = suites.to_vec();
        self
    }

    fn body(&self) -> Vec<u8> {
        let extensions = self
            .extensions
            .iter()
            .map(|(typ, data)| rusty_tls::handrolled::handshake::Extension { typ: *typ, data })
            .collect();
        ClientHello12 {
            version: self.version,
            random: &self.random,
            session_id: &self.session_id,
            cipher_suites: self.suites.clone(),
            compression: &self.compression,
            extensions,
        }
        .encode()
    }

    fn record(&self) -> Vec<u8> {
        record(
            ContentType::Handshake,
            &message(HandshakeType::ClientHello, &self.body()),
        )
    }
}

fn record(typ: ContentType, fragment: &[u8]) -> Vec<u8> {
    let mut out = vec![typ.as_u8(), 3, 3];
    out.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
    out.extend_from_slice(fragment);
    out
}

fn secure_server() -> Server {
    Server::new(Leaf::P256)
}

/// What the server does with one ClientHello record.
fn answer(server: &Server, record: &[u8]) -> Result<Vec<u8>, (ServerError, Option<Vec<u8>>)> {
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let mut handshake = ServerHandshake12::new(&config).expect("server");
    handshake.read_record(record).map_err(|error| {
        let alert = handshake.alert_record(&error);
        // A failure is permanent.
        assert_eq!(
            handshake.read_record(record).err(),
            Some(ServerError::Failed),
            "a handshake that failed must stay failed"
        );
        (error, alert)
    })
}

/// The refusal for a hello, and the description of the alert sent with it.
fn refusal(server: &Server, hello: &Hello) -> (ServerError, AlertDescription) {
    let (error, alert) = answer(server, &hello.record())
        .expect_err("the server accepted a hello it should have refused");
    let alert = alert.expect("a refusal carries an alert");
    assert_eq!(&alert[..3], &[21, 3, 3], "a fatal alert record");
    assert_eq!(alert[5], 2, "fatal");
    (error, AlertDescription(alert[6]))
}

#[test]
fn the_honest_hello_is_accepted() {
    // The control for every edit below: if this failed, the refusals would
    // prove nothing.
    let server = secure_server();
    let reply = answer(&server, &Hello::honest().record()).expect("accepted");
    assert_eq!(reply[0], 22, "the flight is handshake records");
}

#[test]
fn a_hello_that_does_not_offer_tls12_is_refused_with_protocol_version() {
    let server = secure_server();
    let mut old = Hello::honest();
    old.version = 0x0302;
    let (error, alert) = refusal(&server, &old);
    assert_eq!(error, ServerError::NotTls12(0x0302));
    assert_eq!(alert, AlertDescription::PROTOCOL_VERSION);

    // supported_versions without 1.2 is the TLS-1.3-only client.
    let only13 = Hello::honest().with(extension::SUPPORTED_VERSIONS, &[2, 3, 4]);
    assert!(matches!(
        refusal(&server, &only13).0,
        ServerError::NotTls12(_)
    ));

    // Offering both is a client this server can serve as 1.2.
    let both = Hello::honest().with(extension::SUPPORTED_VERSIONS, &[4, 3, 4, 3, 3]);
    assert!(answer(&server, &both.record()).is_ok());

    // A malformed list is malformed, not "no".
    for bad in [&[0u8, 0][..], &[3, 3, 4, 3][..], &[2, 3][..], &[][..]] {
        let hello = Hello::honest().with(extension::SUPPORTED_VERSIONS, bad);
        assert!(
            matches!(refusal(&server, &hello).0, ServerError::Handshake(_)),
            "{bad:?}"
        );
    }
}

#[test]
fn a_hello_without_the_extended_master_secret_is_refused() {
    let server = secure_server();
    for hello in [
        Hello::honest().without(extension::EXTENDED_MASTER_SECRET),
        Hello::honest().with(extension::EXTENDED_MASTER_SECRET, &[0]),
    ] {
        let (error, alert) = refusal(&server, &hello);
        assert_eq!(error, ServerError::MissingExtendedMasterSecret);
        assert_eq!(alert, AlertDescription::HANDSHAKE_FAILURE);
    }
}

#[test]
fn secure_renegotiation_is_required_in_one_of_its_two_forms() {
    let server = secure_server();
    // Neither form.
    let none = Hello::honest().without(extension::RENEGOTIATION_INFO);
    assert_eq!(refusal(&server, &none).0, ServerError::BadRenegotiationInfo);
    // The signalling suite instead of the extension: accepted.
    let mut scsv = none.clone();
    scsv.suites.push(0x00ff);
    assert!(answer(&server, &scsv.record()).is_ok());
    // Both: accepted.
    let mut both = Hello::honest();
    both.suites.push(0x00ff);
    assert!(answer(&server, &both.record()).is_ok());
    // A renegotiated_connection on an initial handshake is never right, and
    // the signalling suite does not excuse it.
    let mut stale = Hello::honest().with(extension::RENEGOTIATION_INFO, &[1, 0x41]);
    stale.suites.push(0x00ff);
    assert_eq!(
        refusal(&server, &stale).0,
        ServerError::BadRenegotiationInfo
    );
}

#[test]
fn compression_must_include_null_and_may_include_more() {
    let server = secure_server();
    let mut only_deflate = Hello::honest();
    only_deflate.compression = vec![1];
    assert!(matches!(
        refusal(&server, &only_deflate).0,
        ServerError::UnacceptableOffer(_)
    ));
    let mut with_null = Hello::honest();
    with_null.compression = vec![1, 0];
    assert!(answer(&server, &with_null.record()).is_ok());
    let mut none = Hello::honest();
    none.compression = vec![];
    assert!(matches!(
        refusal(&server, &none).0,
        ServerError::Handshake(HandshakeError::Empty(_))
    ));
}

#[test]
fn a_hello_with_nothing_in_common_is_refused_with_the_error_that_says_what() {
    let server = secure_server();
    let honest = Hello::honest();

    let (error, alert) = refusal(&server, &honest.clone().suites(&[0x002f, 0x0035, 0xc013]));
    assert_eq!(error, ServerError::NoSharedCipherSuite);
    assert_eq!(alert, AlertDescription::HANDSHAKE_FAILURE);
    // An RSA suite cannot be served by an ECDSA key, however much the client
    // wants it.
    assert_eq!(
        refusal(&server, &honest.clone().suites(&[0xc02f, 0xc030, 0xcca8])).0,
        ServerError::NoSharedCipherSuite
    );
    assert!(answer(&server, &honest.clone().suites(&[0xc02b]).record()).is_ok());

    assert_eq!(
        refusal(
            &server,
            &honest.clone().without(extension::SUPPORTED_GROUPS)
        )
        .0,
        ServerError::NoSharedGroup
    );
    // Offered only a group this server does not implement (ffdhe2048). With an
    // ECDSA key the certificate's curve is what is missing, which is checked
    // first; with an RSA key it is the key exchange.
    let ffdhe_only = honest
        .clone()
        .with(extension::SUPPORTED_GROUPS, &[0, 2, 0x01, 0x00]);
    assert_eq!(
        refusal(&server, &ffdhe_only).0,
        ServerError::NoSharedSignatureScheme
    );
    let rsa = Server::new(Leaf::Rsa);
    assert_eq!(refusal(&rsa, &ffdhe_only).0, ServerError::NoSharedGroup);

    assert_eq!(
        refusal(
            &server,
            &honest.clone().without(extension::SIGNATURE_ALGORITHMS)
        )
        .0,
        ServerError::NoSharedSignatureScheme
    );
    // A scheme this key cannot produce.
    let rsa_only = honest.with(extension::SIGNATURE_ALGORITHMS, &[0, 2, 0x08, 0x04]);
    assert_eq!(
        refusal(&server, &rsa_only).0,
        ServerError::NoSharedSignatureScheme
    );
}

#[test]
fn point_formats_must_include_uncompressed_when_sent() {
    let server = secure_server();
    for bad in [&[1u8, 1][..], &[1, 2], &[0], &[2, 0], &[]] {
        let hello = Hello::honest().with(extension::EC_POINT_FORMATS, bad);
        assert!(
            matches!(
                refusal(&server, &hello).0,
                ServerError::UnacceptableOffer(_)
            ),
            "{bad:?}"
        );
    }
    for good in [&[1u8, 0][..], &[2, 1, 0], &[2, 0, 1]] {
        let hello = Hello::honest().with(extension::EC_POINT_FORMATS, good);
        assert!(answer(&server, &hello.record()).is_ok(), "{good:?}");
    }
    // Absent means uncompressed.
    assert!(answer(
        &server,
        &Hello::honest()
            .without(extension::EC_POINT_FORMATS)
            .record()
    )
    .is_ok());
}

#[test]
fn a_malformed_hello_is_a_decode_error_and_never_a_panic() {
    let server = secure_server();
    let honest = Hello::honest();

    let mut duplicate = honest.clone();
    duplicate.extensions.push(duplicate.extensions[0].clone());
    let (error, alert) = refusal(&server, &duplicate);
    assert!(matches!(
        error,
        ServerError::Handshake(HandshakeError::DuplicateExtension(_))
    ));
    // A message that is shaped wrong (BoGo DuplicateExtensionClient).
    assert_eq!(alert, AlertDescription::DECODE_ERROR);

    let mut long_id = honest.clone();
    long_id.session_id = vec![9; 33];
    assert!(matches!(
        refusal(&server, &long_id).0,
        ServerError::Handshake(HandshakeError::Malformed(_))
    ));
    let mut resume = honest.clone();
    resume.session_id = vec![9; 32];
    let reply = answer(&server, &resume.record()).expect("a resumption offer is just ignored");
    // The ServerHello's session_id is empty: this server never resumes.
    assert_eq!(reply[5 + 4 + 2 + 32], 0, "the ServerHello session id");

    assert!(matches!(
        refusal(&server, &honest.clone().suites(&[])).0,
        ServerError::Handshake(HandshakeError::Empty(_))
    ));

    // Every prefix of a valid hello body, as a whole message: none may panic,
    // and all but the one that happens to be a valid extension-less hello
    // must be refused.
    let body = honest.body();
    let mut accepted = 0;
    for cut in 0..body.len() {
        let record = record(
            ContentType::Handshake,
            &message(HandshakeType::ClientHello, &body[..cut]),
        );
        if answer(&server, &record).is_ok() {
            accepted += 1;
        }
    }
    // A hello with no extensions block parses, and is then refused later for
    // the missing extended master secret, so nothing short of whole is served.
    assert_eq!(accepted, 0);
}

#[test]
fn an_unbounded_handshake_message_is_cut_off() {
    let server = secure_server();
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let mut handshake = ServerHandshake12::new(&config).expect("server");
    // A ClientHello header that promises 16 MiB, then plenty of it.
    let mut first = vec![1, 0xff, 0xff, 0xff];
    first.resize(16_384, 0);
    let mut sent = 0;
    let error = loop {
        let fragment = if sent == 0 {
            first.clone()
        } else {
            vec![0; 16_384]
        };
        match handshake.read_record(&record(ContentType::Handshake, &fragment)) {
            Ok(_) => sent += 1,
            Err(error) => break error,
        }
        assert!(sent < 16, "the server buffered without limit");
    };
    assert_eq!(error, ServerError::HandshakeTooLarge);
    assert_eq!(error.alert(), Some(AlertDescription::ILLEGAL_PARAMETER));
}

#[test]
fn nothing_is_accepted_out_of_order() {
    let server = secure_server();
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let fresh = || ServerHandshake12::new(&config).expect("server");

    // Before any hello.
    for (typ, fragment, want) in [
        (ContentType::ApplicationData, &[1u8][..], "content"),
        (ContentType::ChangeCipherSpec, &[1], "ccs"),
        (
            ContentType::Handshake,
            &[20, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            "message",
        ),
    ] {
        let error = fresh()
            .read_record(&record(typ, fragment))
            .expect_err("refused");
        match want {
            "content" => assert!(matches!(error, ServerError::UnexpectedContentType(_))),
            "ccs" => assert_eq!(error, ServerError::UnexpectedChangeCipherSpec),
            _ => assert!(matches!(error, ServerError::UnexpectedMessage { .. })),
        }
    }
    // A fatal alert from the client ends it; a warning does not.
    assert!(matches!(
        fresh().read_record(&record(ContentType::Alert, &[2, 40])),
        Err(ServerError::PeerAlert(_))
    ));
    assert_eq!(
        fresh().read_record(&record(ContentType::Alert, &[1, 100])),
        Ok(vec![])
    );
    assert!(fresh()
        .read_record(&record(ContentType::Alert, &[2]))
        .is_err());
}

#[test]
fn the_client_flight_is_accepted_only_whole_and_in_order() {
    let server = secure_server();
    with_configs(&server, CipherSuite12::SUPPORTED, |sc, cc| {
        // Control.
        let at = at_client_flight(sc, cc);
        let _ = finish(at);

        // ChangeCipherSpec before the ClientKeyExchange (CVE-2014-0224's shape).
        let mut at = at_client_flight(sc, cc);
        let error = at.server.read_record(&at.flight[1]).expect_err("refused");
        assert_eq!(error, ServerError::UnexpectedChangeCipherSpec);
        assert_eq!(error.alert(), Some(AlertDescription::UNEXPECTED_MESSAGE));

        // The Finished before the ChangeCipherSpec.
        let mut at = at_client_flight(sc, cc);
        at.server.read_record(&at.flight[0]).expect("hs");
        let error = at.server.read_record(&at.flight[2]).expect_err("refused");
        assert!(
            matches!(error, ServerError::UnexpectedContentType(_)),
            "{error:?}"
        );

        // The handshake record twice.
        let mut at = at_client_flight(sc, cc);
        at.server.read_record(&at.flight[0]).expect("hs");
        let error = at.server.read_record(&at.flight[0]).expect_err("refused");
        assert!(
            matches!(error, ServerError::UnexpectedContentType(_)),
            "{error:?}"
        );

        // A ChangeCipherSpec that is not exactly 0x01.
        for body in [&[0u8][..], &[2], &[1, 1], &[]] {
            let mut at = at_client_flight(sc, cc);
            at.server.read_record(&at.flight[0]).expect("hs");
            let error = at
                .server
                .read_record(&record(ContentType::ChangeCipherSpec, body))
                .expect_err("refused");
            assert_eq!(error, ServerError::UnexpectedChangeCipherSpec, "{body:?}");
        }

        // No ChangeCipherSpec at all, application data instead.
        let mut at = at_client_flight(sc, cc);
        at.server.read_record(&at.flight[0]).expect("hs");
        let error = at
            .server
            .read_record(&record(ContentType::ApplicationData, b"early"))
            .expect_err("refused");
        assert!(
            matches!(error, ServerError::UnexpectedContentType(_)),
            "{error:?}"
        );
    });
}

/// Every bit of the client's flight, flipped one at a time: each must be
/// refused, except the minor-version octet of the two plaintext records (a hint
/// per RFC 5246 appendix E), whose flips must be accepted — saying so keeps the
/// exception from growing.
#[test]
fn every_one_bit_corruption_of_the_client_flight_is_refused() {
    let server = secure_server();
    with_configs(&server, CipherSuite12::SUPPORTED, |sc, cc| {
        // Sizes are fixed by the configuration; the bytes are not (every
        // handshake has fresh randoms and keys), so each flip is applied to the
        // flight of the very handshake it is fed to.
        let lengths: Vec<usize> = at_client_flight(sc, cc)
            .flight
            .iter()
            .map(Vec::len)
            .collect();
        let (mut tested, mut ignored) = (0, 0);
        for (index, length) in lengths.iter().enumerate() {
            for byte in 0..*length {
                for bit in 0..8 {
                    let mut at = at_client_flight(sc, cc);
                    at.flight[index][byte] ^= 1 << bit;
                    let accepted = at
                        .flight
                        .iter()
                        .all(|record| at.server.read_record(record).is_ok())
                        && at.server.is_finished();
                    tested += 1;
                    // Only the minor-version octet of the two plaintext records.
                    let hint = (index == 0 || index == 1) && byte == 2;
                    if accepted {
                        assert!(hint, "flight {index} byte {byte} bit {bit} was accepted");
                        ignored += 1;
                    }
                }
            }
        }
        assert!(tested > 700, "the sweep ran {tested} cases");
        assert_eq!(
            ignored, 16,
            "exactly the 2 x 8 version-hint flips are ignored"
        );
    });
}

#[test]
fn a_finished_handshake_reads_no_more_handshake_records() {
    let server = secure_server();
    with_configs(&server, CipherSuite12::SUPPORTED, |sc, cc| {
        let mut at = at_client_flight(sc, cc);
        for record in &at.flight {
            at.server.read_record(record).expect("accepted");
        }
        assert!(at.server.is_finished());
        // Done is terminal: the handshake object reads nothing more, and a
        // caller that mistakenly tries gets no alert to send.
        let error = at.server.read_record(&at.flight[0]).expect_err("refused");
        assert_eq!(error, ServerError::Failed);
        assert_eq!(at.server.alert_record(&error), None);
    });
}

// ---------------------------------------------------------------------------
// Client authentication, refused in pieces
// ---------------------------------------------------------------------------

/// A rustls client's flight with its handshake messages taken apart, so one can
/// be removed, replaced or damaged and the rest put back.
struct Pieces {
    /// Handshake messages before the `ChangeCipherSpec`, each whole.
    messages: Vec<Vec<u8>>,
    /// The `ChangeCipherSpec` and the protected `Finished` records, as sent.
    rest: Vec<Vec<u8>>,
}

impl Pieces {
    fn of(records: Vec<Vec<u8>>) -> Self {
        let mut handshake = Vec::new();
        let mut rest = Vec::new();
        for record in records {
            if rest.is_empty() && record[0] == 22 {
                handshake.extend_from_slice(&record[5..]);
            } else {
                rest.push(record);
            }
        }
        let messages = messages(&handshake)
            .expect("rustls sends well-formed messages")
            .iter()
            .map(|m| m.encoded.to_vec())
            .collect();
        Self { messages, rest }
    }

    fn kind(message: &[u8]) -> HandshakeType {
        HandshakeType::from_u8(message[0])
    }

    fn without(mut self, typ: HandshakeType) -> Self {
        self.messages.retain(|m| Self::kind(m) != typ);
        self
    }

    fn replacing(mut self, typ: HandshakeType, body: &[u8]) -> Self {
        for m in &mut self.messages {
            if Self::kind(m) == typ {
                *m = message(typ, body);
            }
        }
        self
    }

    fn damaging_last_byte_of(mut self, typ: HandshakeType) -> Self {
        for m in &mut self.messages {
            if Self::kind(m) == typ {
                *m.last_mut().expect("non-empty") ^= 0x01;
            }
        }
        self
    }

    fn records(&self) -> Vec<Vec<u8>> {
        let mut out = vec![record(ContentType::Handshake, &self.messages.concat())];
        out.extend(self.rest.iter().cloned());
        out
    }
}

/// Run a server with client authentication against a rustls client holding
/// `identity`, up to the point where the client's flight is in hand.
fn with_client_flight<R>(
    server: &Server,
    trusted: &[&Pki],
    required: bool,
    identity: &Pki,
    check: impl FnOnce(ServerHandshake12<'_>, Pieces) -> R,
) -> R {
    let root_ders: Vec<Vec<u8>> = trusted.iter().map(|p| p.root_der.clone()).collect();
    let anchors: Vec<TrustAnchor<'_>> = root_ders.iter().map(|d| anchor(d)).collect();
    let auth = ClientAuth {
        anchors: &anchors,
        path: options(),
        required,
    };
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, Some(&auth));
    let mut client = rustls_client_with(
        &server.pki.root_der,
        Arc::new(rustls::crypto::ring::default_provider()),
        &[&rustls::version::TLS12],
        Some(identity),
    );
    let mut handshake = ServerHandshake12::new(&config).expect("server");
    let hello = pump_client(&mut client, &[]).0;
    let mut flight1 = Vec::new();
    for record in take_records(&mut hello.clone()) {
        flight1.extend(
            handshake
                .read_record(&record)
                .expect("the hello is accepted"),
        );
    }
    let (bytes, refusal) = pump_client(&mut client, &flight1);
    assert_eq!(refusal, None);
    check(handshake, Pieces::of(take_records(&mut bytes.clone())))
}

fn feed(server: &mut ServerHandshake12<'_>, records: &[Vec<u8>]) -> Result<(), ServerError> {
    for record in records {
        server.read_record(record)?;
    }
    Ok(())
}

#[test]
fn the_client_authentication_refusals_have_a_control_that_passes() {
    let server = Server::new(Leaf::P256);
    let client = pki(Leaf::P256, "client.example");
    with_client_flight(&server, &[&client], true, &client, |mut srv, pieces| {
        assert_eq!(
            pieces
                .messages
                .iter()
                .map(|m| Pieces::kind(m))
                .collect::<Vec<_>>(),
            [
                HandshakeType::Certificate,
                HandshakeType::ClientKeyExchange,
                HandshakeType::CertificateVerify
            ]
        );
        // Taken apart and put back, the flight is still accepted.
        feed(&mut srv, &pieces.records()).expect("the rebuilt flight is accepted");
        assert!(srv.is_finished());
    });
}

#[test]
fn a_certificate_without_a_certificate_verify_is_refused() {
    // The point of CertificateVerify: presenting a certificate proves nothing.
    // A client that sends one and skips the proof must not be authenticated.
    let server = Server::new(Leaf::P256);
    let client = pki(Leaf::P256, "client.example");
    with_client_flight(&server, &[&client], true, &client, |mut srv, pieces| {
        let error = feed(
            &mut srv,
            &pieces.without(HandshakeType::CertificateVerify).records(),
        )
        .expect_err("refused");
        assert_eq!(error, ServerError::UnexpectedChangeCipherSpec);
    });
}

#[test]
fn a_certificate_verify_that_does_not_verify_is_refused() {
    let server = Server::new(Leaf::P256);
    let client = pki(Leaf::P256, "client.example");
    with_client_flight(&server, &[&client], true, &client, |mut srv, pieces| {
        let error = feed(
            &mut srv,
            &pieces
                .damaging_last_byte_of(HandshakeType::CertificateVerify)
                .records(),
        )
        .expect_err("refused");
        assert!(
            matches!(error, ServerError::ClientCertificateVerify(_)),
            "{error:?}"
        );
        assert_eq!(error.alert(), Some(AlertDescription::DECRYPT_ERROR));
    });
}

#[test]
fn someone_elses_certificate_with_my_signature_is_refused() {
    // Both clients are trusted. The flight is client A's, with client B's
    // (valid, trusted) certificate swapped in: the chain validates, and the
    // signature, made with A's key, must not verify under B's.
    let server = Server::new(Leaf::P256);
    let a = pki(Leaf::P256, "a.example");
    let b = pki(Leaf::P256, "b.example");
    with_client_flight(&server, &[&a, &b], true, &a, |mut srv, pieces| {
        let swapped = pieces.replacing(
            HandshakeType::Certificate,
            &Certificate12::encode(&[&b.leaf_der]),
        );
        let error = feed(&mut srv, &swapped.records()).expect_err("refused");
        assert!(
            matches!(error, ServerError::ClientCertificateVerify(_)),
            "{error:?}"
        );
    });
}

#[test]
fn the_certificate_message_cannot_be_skipped_when_a_certificate_was_requested() {
    let server = Server::new(Leaf::P256);
    let client = pki(Leaf::P256, "client.example");
    with_client_flight(&server, &[&client], false, &client, |mut srv, pieces| {
        let error = feed(
            &mut srv,
            &pieces.without(HandshakeType::Certificate).records(),
        )
        .expect_err("refused");
        assert!(
            matches!(
                error,
                ServerError::UnexpectedMessage {
                    expected: "Certificate",
                    ..
                }
            ),
            "{error:?}"
        );
    });
}

#[test]
fn an_empty_certificate_does_not_carry_a_certificate_verify_with_it() {
    // "I have none" followed by a proof of possession is incoherent.
    let server = Server::new(Leaf::P256);
    let client = pki(Leaf::P256, "client.example");
    with_client_flight(&server, &[&client], false, &client, |mut srv, pieces| {
        let error = feed(
            &mut srv,
            &pieces
                .replacing(HandshakeType::Certificate, &Certificate12::encode(&[]))
                .records(),
        )
        .expect_err("refused");
        assert!(
            matches!(
                error,
                ServerError::UnexpectedMessage {
                    expected: "ChangeCipherSpec",
                    got: HandshakeType::CertificateVerify
                }
            ),
            "{error:?}"
        );
    });
}

#[test]
fn malformed_client_certificates_are_refused_by_name() {
    let server = Server::new(Leaf::P256);
    let client = pki(Leaf::P256, "client.example");
    let cases: [(&str, Vec<u8>); 3] = [
        ("an empty entry", vec![0, 0, 3, 0, 0, 0]),
        (
            "garbage DER",
            Certificate12::encode(&[b"\x30\x03not a certificate"]),
        ),
        ("trailing bytes", {
            let mut body = Certificate12::encode(&[&client.leaf_der]);
            body.push(0);
            body
        }),
    ];
    for (name, body) in cases {
        with_client_flight(&server, &[&client], true, &client, |mut srv, pieces| {
            let error = feed(
                &mut srv,
                &pieces
                    .replacing(HandshakeType::Certificate, &body)
                    .records(),
            )
            .err()
            .unwrap_or_else(|| panic!("{name} was accepted"));
            assert!(
                matches!(
                    error,
                    ServerError::Handshake(_) | ServerError::MalformedClientCertificate(_)
                ),
                "{name}: {error:?}"
            );
        });
    }
}

// ---------------------------------------------------------------------------
// Whatever arrives
// ---------------------------------------------------------------------------

/// A small deterministic generator, so a failure is reproducible.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() >> 24) as u8).collect()
    }
}

/// Random records, random handshake bodies and mutated hellos: all must end in
/// a refusal or an acceptance, never a panic, and a refusal must be final.
#[test]
fn arbitrary_input_never_panics_and_failure_is_final() {
    let server = secure_server();
    let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let honest = Hello::honest().record();
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);

    for round in 0..3000 {
        let mut handshake = ServerHandshake12::new(&config).expect("server");
        let record = match round % 4 {
            // Random type, random body.
            0 => {
                let len = (rng.next() % 200) as usize;
                let mut r = vec![(rng.next() % 40) as u8, 3, 3];
                r.extend_from_slice(&(len as u16).to_be_bytes());
                r.extend(rng.bytes(len));
                r
            }
            // A handshake record with a random message in it.
            1 => {
                let len = (rng.next() % 300) as usize;
                let typ = (rng.next() % 30) as u8;
                let mut body = vec![typ, 0, (len >> 8) as u8, len as u8];
                body.extend(rng.bytes(len));
                record(ContentType::Handshake, &body)
            }
            // The honest hello with a few random bytes overwritten.
            2 => {
                let mut r = honest.clone();
                for _ in 0..=(rng.next() % 4) {
                    let at = 5 + (rng.next() as usize % (r.len() - 5));
                    r[at] = (rng.next() >> 16) as u8;
                }
                r
            }
            // The honest hello cut short, with the length fixed up.
            _ => {
                let keep = 5 + (rng.next() as usize % (honest.len() - 5));
                let mut r = honest[..keep].to_vec();
                let len = (keep - 5) as u16;
                r[3..5].copy_from_slice(&len.to_be_bytes());
                r
            }
        };
        match handshake.read_record(&record) {
            Ok(_) => {}
            Err(error) => {
                let _ = handshake.alert_record(&error);
                assert_eq!(handshake.read_record(&honest), Err(ServerError::Failed));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// A scripted client, for what only a correctly keyed peer can do
// ---------------------------------------------------------------------------

/// A TLS 1.2 client written from the public pieces of this crate (key exchange,
/// key schedule, record layer) and nothing else, so it can send what the real
/// client never would: a `Finished` that is perfectly encrypted and wrong, a
/// handshake message trailing it in the same record, a renegotiation request.
/// The live peers above are what show it is a faithful client; its control
/// test shows a correct flight from it completes.
struct Scripted {
    hash: Hash,
    master: MasterSecret,
    sealer: Sealer,
    opener: Opener,
    /// Everything up to and including `ClientKeyExchange`.
    transcript: Vec<u8>,
    /// What the server chose.
    suite: CipherSuite12,
    /// The server's first-flight extensions, as `(type, data)`.
    server_extensions: Vec<(u16, Vec<u8>)>,
    server_hello_session_id: Vec<u8>,
    server_random: Vec<u8>,
    server_public: Vec<u8>,
    cke: Vec<u8>,
}

impl Scripted {
    /// Send `hello`, read the server's flight and key up.
    fn start(server: &mut ServerHandshake12<'_>, hello: &Hello) -> Self {
        let mut flight = server
            .read_record(&hello.record())
            .expect("the server accepts the hello");
        let mut fragments = Vec::new();
        for record in take_records(&mut flight) {
            fragments.extend_from_slice(&record[5..]);
        }
        let parsed = messages(&fragments).expect("the flight parses");

        let mut transcript = message(HandshakeType::ClientHello, &hello.body());
        let mut chosen = None;
        let mut ske = None;
        for m in &parsed {
            transcript.extend_from_slice(m.encoded);
            match m.typ {
                HandshakeType::ServerHello => chosen = Some(m.body.to_vec()),
                HandshakeType::ServerKeyExchange => ske = Some(m.body.to_vec()),
                _ => {}
            }
        }
        let body = chosen.expect("a ServerHello");
        let server_hello = ServerHello12::parse(&body).expect("parses");
        let suite = CipherSuite12(server_hello.cipher_suite);
        let (aead, hash, _) = suite.parts().expect("a suite we implement");
        let ske_body = ske.expect("a ServerKeyExchange");
        let ske = ServerKeyExchange::parse(&ske_body).expect("parses");
        let group = NamedGroup::from_u16(ske.named_curve).expect("a group we implement");

        let kx = KeyExchange::generate(group).expect("keygen");
        let cke = message(
            HandshakeType::ClientKeyExchange,
            &encode_client_key_exchange(kx.public_key()),
        );
        transcript.extend_from_slice(&cke);
        let pre_master = kx.agree(ske.public, |s| s.to_vec()).expect("agree");
        let master =
            extended_master_secret(hash, &pre_master, &hash.hash(&transcript)).expect("master");
        let array = |bytes: &[u8]| -> [u8; 32] { bytes.try_into().expect("32 octets") };
        let keys = key_block(
            hash,
            aead,
            &master,
            &array(&hello.random),
            &array(server_hello.random),
        );
        Self {
            hash,
            master,
            sealer: Sealer::new(aead, &keys.client_write_key, &keys.client_write_iv)
                .expect("sealer"),
            opener: Opener::new(aead, &keys.server_write_key, &keys.server_write_iv)
                .expect("opener"),
            transcript,
            suite,
            server_extensions: server_hello
                .extensions
                .iter()
                .map(|e| (e.typ, e.data.to_vec()))
                .collect(),
            server_hello_session_id: server_hello.session_id.to_vec(),
            server_random: server_hello.random.to_vec(),
            server_public: ske.public.to_vec(),
            cke,
        }
    }

    fn cke_record(&self) -> Vec<u8> {
        record(ContentType::Handshake, &self.cke)
    }

    fn ccs_record() -> Vec<u8> {
        record(ContentType::ChangeCipherSpec, &[1])
    }

    /// The `verify_data` this client would send, or that it would expect from
    /// the server's side with `side = Server` over `transcript`.
    fn verify_data(&self, side: Side, transcript: &[u8]) -> Vec<u8> {
        finished_verify_data(self.hash, &self.master, side, &self.hash.hash(transcript))
            .expect("verify data")
            .to_vec()
    }

    /// A protected record holding `Finished(data)` followed by `trailing`.
    fn finished_record(&mut self, data: &[u8], trailing: &[u8]) -> Vec<u8> {
        let mut fragment = message(HandshakeType::Finished, data);
        fragment.extend_from_slice(trailing);
        self.sealer
            .seal(ContentType::Handshake, &fragment)
            .expect("seal")
    }

    /// Read the server's `ChangeCipherSpec` and `Finished`, and check the
    /// latter is what the server owed: a MAC over everything, with this
    /// client's own Finished included.
    fn check_server_finish(&mut self, mut reply: Vec<u8>, client_verify: &[u8]) {
        let records = take_records(&mut reply);
        assert_eq!(records.len(), 2, "ChangeCipherSpec and Finished");
        assert_eq!(records[0], Self::ccs_record());
        let opened = self.opener.open(&records[1]).expect("the Finished opens");
        assert_eq!(opened.typ, ContentType::Handshake);
        let parsed = messages(&opened.fragment).expect("one message");
        assert_eq!(parsed.len(), 1);
        let mut transcript = self.transcript.clone();
        transcript.extend(message(HandshakeType::Finished, client_verify));
        assert!(verify_finished(
            self.hash,
            &self.master,
            Side::Server,
            &self.hash.hash(&transcript),
            parsed[0].body
        ));
    }
}

fn scripted_server(server: &Server) -> (ServerConfig12<'_>, Hello) {
    (
        server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None),
        Hello::honest(),
    )
}

mod scripted {
    use super::*;

    /// Every scripted refusal below is only meaningful because this passes.
    #[test]
    fn a_correct_scripted_flight_completes_and_the_servers_finished_verifies() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let mut srv = ServerHandshake12::new(&config).expect("server");
        let mut client = Scripted::start(&mut srv, &hello);

        srv.read_record(&client.cke_record()).expect("cke");
        srv.read_record(&Scripted::ccs_record()).expect("ccs");
        let data = client.verify_data(Side::Client, &client.transcript.clone());
        let reply = srv
            .read_record(&client.finished_record(&data, &[]))
            .expect("the Finished verifies");
        client.check_server_finish(reply, &data);
        assert!(srv.is_finished());
    }

    #[test]
    fn what_the_server_chose_and_said() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let mut srv = ServerHandshake12::new(&config).expect("server");
        let client = Scripted::start(&mut srv, &hello);

        // The server's own first preference that the key can serve.
        assert_eq!(
            client.suite,
            CipherSuite12::ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
        );
        assert!(client.server_hello_session_id.is_empty(), "no resumption");
        assert_eq!(client.server_random.len(), 32);
        let types: Vec<u16> = client.server_extensions.iter().map(|(t, _)| *t).collect();
        // Exactly what was offered and agreed, in a fixed order: nothing the
        // client did not ask for (RFC 5246 §7.4.1.4).
        assert!(types.contains(&extension::EXTENDED_MASTER_SECRET));
        assert!(types.contains(&extension::RENEGOTIATION_INFO));
        assert!(
            types.contains(&extension::EC_POINT_FORMATS),
            "the client sent it"
        );
        assert_eq!(types.len(), 3, "{types:?}");
        let data = |typ| {
            client
                .server_extensions
                .iter()
                .find(|(t, _)| *t == typ)
                .map(|(_, d)| d.clone())
        };
        assert_eq!(data(extension::RENEGOTIATION_INFO), Some(vec![0]));
        assert_eq!(data(extension::EXTENDED_MASTER_SECRET), Some(vec![]));
        assert_eq!(data(extension::EC_POINT_FORMATS), Some(vec![1, 0]));
    }

    #[test]
    fn point_formats_are_echoed_only_when_offered() {
        let server = secure_server();
        let config = server.config(CipherSuite12::SUPPORTED, ALL_GROUPS, None);
        let mut srv = ServerHandshake12::new(&config).expect("server");
        let hello = Hello::honest().without(extension::EC_POINT_FORMATS);
        let client = Scripted::start(&mut srv, &hello);
        assert!(client
            .server_extensions
            .iter()
            .all(|(t, _)| *t != extension::EC_POINT_FORMATS));
    }

    #[test]
    fn two_handshakes_never_share_a_random_or_an_ephemeral_key() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            let mut srv = ServerHandshake12::new(&config).expect("server");
            let client = Scripted::start(&mut srv, &hello);
            assert!(
                seen.insert(client.server_random.clone()),
                "a repeated random"
            );
            assert!(seen.insert(client.server_public.clone()), "a repeated key");
        }
    }

    /// The `Finished` is the proof both ends derived the same keys from the
    /// same transcript. Each wrong one is perfectly encrypted, so only the
    /// `Finished` check itself can catch it.
    #[test]
    fn a_wrong_finished_is_refused_however_well_it_is_encrypted() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let case = |mutate: &dyn Fn(&mut Scripted, Vec<u8>) -> Vec<u8>| {
            let mut srv = ServerHandshake12::new(&config).expect("server");
            let mut client = Scripted::start(&mut srv, &hello);
            srv.read_record(&client.cke_record()).expect("cke");
            srv.read_record(&Scripted::ccs_record()).expect("ccs");
            let honest = client.verify_data(Side::Client, &client.transcript.clone());
            let record = mutate(&mut client, honest);
            srv.read_record(&record)
                .expect_err("a wrong Finished is refused")
        };

        for bit in 0..96 {
            let error = case(&|c, mut data| {
                data[bit / 8] ^= 1 << (bit % 8);
                c.finished_record(&data, &[])
            });
            assert_eq!(error, ServerError::BadFinished, "bit {bit}");
            assert_eq!(error.alert(), Some(AlertDescription::DECRYPT_ERROR));
        }
        // The server's own label: a reflection of what the server would send.
        let reflected = case(&|c, _| {
            let data = c.verify_data(Side::Server, &c.transcript.clone());
            c.finished_record(&data, &[])
        });
        assert_eq!(reflected, ServerError::BadFinished);
        // Over a transcript missing the ClientKeyExchange.
        let short = case(&|c, _| {
            let without = c.transcript[..c.transcript.len() - c.cke.len()].to_vec();
            let data = c.verify_data(Side::Client, &without);
            c.finished_record(&data, &[])
        });
        assert_eq!(short, ServerError::BadFinished);
        // The wrong length is a Finished that does not verify, not a message
        // that does not parse (BoGo `TrailingMessageData-ClientFinished`).
        for len in [0usize, 11, 13, 32] {
            let error = case(&|c, honest| {
                let mut data = honest;
                data.resize(len, 0);
                c.finished_record(&data, &[])
            });
            assert_eq!(error, ServerError::BadFinished, "length {len}");
        }
    }

    #[test]
    fn nothing_may_follow_the_finished_in_its_record() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let case = |trailing: &[u8]| {
            let mut srv = ServerHandshake12::new(&config).expect("server");
            let mut client = Scripted::start(&mut srv, &hello);
            srv.read_record(&client.cke_record()).expect("cke");
            srv.read_record(&Scripted::ccs_record()).expect("ccs");
            let data = client.verify_data(Side::Client, &client.transcript.clone());
            let record = client.finished_record(&data, trailing);
            (srv.read_record(&record), srv.is_finished())
        };
        // A whole message (a HelloRequest), and a partial header.
        for trailing in [&[0u8, 0, 0, 0][..], &[20, 0], &[0]] {
            let (result, finished) = case(trailing);
            assert!(
                matches!(
                    result,
                    Err(ServerError::UnexpectedMessage {
                        expected: "nothing after Finished",
                        ..
                    })
                ),
                "{trailing:?}: {result:?}"
            );
            assert!(!finished);
        }
        assert!(case(&[]).0.is_ok(), "the control");
    }

    #[test]
    fn a_change_cipher_spec_with_handshake_bytes_half_read_is_refused() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let mut srv = ServerHandshake12::new(&config).expect("server");
        let client = Scripted::start(&mut srv, &hello);

        // The ClientKeyExchange, then the first two octets of another message.
        let mut fragment = client.cke.clone();
        fragment.extend_from_slice(&[20, 0]);
        srv.read_record(&record(ContentType::Handshake, &fragment))
            .expect("buffered");
        let error = srv
            .read_record(&Scripted::ccs_record())
            .expect_err("a ChangeCipherSpec in the middle of a message");
        assert_eq!(error, ServerError::UnexpectedChangeCipherSpec);
    }

    #[test]
    fn a_renegotiation_request_after_the_handshake_is_refused_and_the_connection_carries_on() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let mut srv = ServerHandshake12::new(&config).expect("server");
        let mut client = Scripted::start(&mut srv, &hello);
        srv.read_record(&client.cke_record()).expect("cke");
        srv.read_record(&Scripted::ccs_record()).expect("ccs");
        let data = client.verify_data(Side::Client, &client.transcript.clone());
        let reply = srv
            .read_record(&client.finished_record(&data, &[]))
            .expect("finished");
        client.check_server_finish(reply, &data);
        let mut connection = srv.into_connection().expect("connection");

        // A whole ClientHello inside the encrypted connection: refused, by a
        // warning of exactly no_renegotiation(100), and nothing else happens.
        let hello_body = Hello::honest().body();
        let request = message(HandshakeType::ClientHello, &hello_body);
        let sealed = client
            .sealer
            .seal(ContentType::Handshake, &request)
            .expect("seal");
        let Incoming12::Reply(mut alert) = connection.read(&sealed).expect("answered") else {
            panic!("expected a reply");
        };
        let opened = client
            .opener
            .open(&take_records(&mut alert)[0])
            .expect("opens");
        assert_eq!(opened.typ, ContentType::Alert);
        assert_eq!(opened.fragment, [1, 100]);

        // And the connection still works, both ways.
        let sealed = client
            .sealer
            .seal(ContentType::ApplicationData, b"still here")
            .expect("seal");
        assert_eq!(
            connection.read(&sealed).expect("read"),
            Incoming12::Application(b"still here".to_vec())
        );
        let mut wire = connection.write(b"yes").expect("write");
        let opened = client
            .opener
            .open(&take_records(&mut wire)[0])
            .expect("opens");
        assert_eq!(opened.fragment, b"yes");
    }

    #[test]
    fn only_a_whole_client_hello_counts_as_a_renegotiation_request() {
        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let body = Hello::honest().body();
        let whole = message(HandshakeType::ClientHello, &body);
        let mut short = whole.clone();
        short.truncate(short.len() - 1);
        let mut long = whole.clone();
        long.push(0);

        let cases: [(&str, Vec<u8>); 7] = [
            // A HelloRequest is the *server's* way to ask; a client sending one
            // is not asking to renegotiate.
            ("a HelloRequest", vec![0, 0, 0, 0]),
            ("a short hello", short),
            ("a hello and a stray octet", long),
            ("just the type", vec![1]),
            ("a header only", vec![1, 0, 0]),
            ("a Finished", message(HandshakeType::Finished, &[0; 12])),
            ("nothing", vec![]),
        ];
        for (name, fragment) in cases {
            let mut srv = ServerHandshake12::new(&config).expect("server");
            let mut client = Scripted::start(&mut srv, &hello);
            srv.read_record(&client.cke_record()).expect("cke");
            srv.read_record(&Scripted::ccs_record()).expect("ccs");
            let data = client.verify_data(Side::Client, &client.transcript.clone());
            let reply = srv
                .read_record(&client.finished_record(&data, &[]))
                .expect("finished");
            client.check_server_finish(reply, &data);
            let mut connection = srv.into_connection().expect("connection");

            let sealed = client
                .sealer
                .seal(ContentType::Handshake, &fragment)
                .expect("seal");
            let result = connection.read(&sealed);
            assert!(result.is_err(), "{name}: {result:?}");
            // A failure is final.
            let sealed = client
                .sealer
                .seal(ContentType::ApplicationData, b"x")
                .expect("seal");
            assert!(
                matches!(connection.read(&sealed), Err(ClientError::Failed)),
                "{name}"
            );
        }
    }

    /// BoGo: SendEmptyRecords (32 pass, 33 fail), SendWarningAlerts (4 pass,
    /// 5 fail), and the two counted separately so neither hides behind the
    /// other.
    #[test]
    fn floods_of_empty_records_and_warnings_are_cut_off_on_an_established_connection() {
        use rusty_tls::handrolled::limits::{Flood, MAX_EMPTY_RECORDS, MAX_WARNING_ALERTS};

        let server = secure_server();
        let (config, hello) = scripted_server(&server);
        let established = || {
            let mut srv = ServerHandshake12::new(&config).expect("server");
            let mut client = Scripted::start(&mut srv, &hello);
            srv.read_record(&client.cke_record()).expect("cke");
            srv.read_record(&Scripted::ccs_record()).expect("ccs");
            let data = client.verify_data(Side::Client, &client.transcript.clone());
            let reply = srv
                .read_record(&client.finished_record(&data, &[]))
                .expect("finished");
            client.check_server_finish(reply, &data);
            (client, srv.into_connection().expect("connection"))
        };

        // Empty records: 32 are fine, data resets, the 33rd of a run is not.
        let (mut client, mut connection) = established();
        for _ in 0..MAX_EMPTY_RECORDS {
            let r = client
                .sealer
                .seal(ContentType::ApplicationData, &[])
                .expect("seal");
            assert_eq!(
                connection.read(&r).expect("within the limit"),
                Incoming12::Application(vec![])
            );
        }
        let r = client
            .sealer
            .seal(ContentType::ApplicationData, b"x")
            .expect("seal");
        connection.read(&r).expect("data");
        for _ in 0..MAX_EMPTY_RECORDS {
            let r = client
                .sealer
                .seal(ContentType::ApplicationData, &[])
                .expect("seal");
            connection.read(&r).expect("a fresh run");
        }
        let r = client
            .sealer
            .seal(ContentType::ApplicationData, &[])
            .expect("seal");
        assert!(matches!(
            connection.read(&r),
            Err(ClientError::Flood(Flood::EmptyRecords))
        ));

        // Warnings: 4 are fine, the 5th is not, empty records do not reset them.
        let (mut client, mut connection) = established();
        for i in 0..MAX_WARNING_ALERTS {
            let r = client
                .sealer
                .seal(ContentType::Alert, &[1, 100])
                .expect("seal");
            assert_eq!(
                connection.read(&r).expect("advisory"),
                Incoming12::Handled,
                "{i}"
            );
            let r = client
                .sealer
                .seal(ContentType::ApplicationData, &[])
                .expect("seal");
            connection.read(&r).expect("empty");
        }
        let r = client
            .sealer
            .seal(ContentType::Alert, &[1, 100])
            .expect("seal");
        assert!(matches!(
            connection.read(&r),
            Err(ClientError::Flood(Flood::WarningAlerts))
        ));
    }

    /// Before keys exist the same floods apply to the handshake's own reader.
    #[test]
    fn floods_are_cut_off_during_the_handshake_too() {
        use rusty_tls::handrolled::limits::{Flood, MAX_EMPTY_RECORDS, MAX_WARNING_ALERTS};

        let server = secure_server();
        let (config, _) = scripted_server(&server);

        let mut srv = ServerHandshake12::new(&config).expect("server");
        for _ in 0..MAX_EMPTY_RECORDS {
            srv.read_record(&record(ContentType::Handshake, &[]))
                .expect("within the limit");
        }
        assert_eq!(
            srv.read_record(&record(ContentType::Handshake, &[])),
            Err(ServerError::Flood(Flood::EmptyRecords))
        );

        let mut srv = ServerHandshake12::new(&config).expect("server");
        for _ in 0..MAX_WARNING_ALERTS {
            srv.read_record(&record(ContentType::Alert, &[1, 100]))
                .expect("advisory");
        }
        assert_eq!(
            srv.read_record(&record(ContentType::Alert, &[1, 100])),
            Err(ServerError::Flood(Flood::WarningAlerts))
        );
    }
}
