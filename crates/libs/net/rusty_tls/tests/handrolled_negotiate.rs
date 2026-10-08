//! Version negotiation and downgrade protection — stage 4b-v.
//!
//! One endpoint that speaks TLS 1.3 and TLS 1.2 must choose, and a wrong choice
//! is a security failure, not a compatibility one. Evidence, in order:
//!
//! 1. **Live rustls on both sides** picks the same version this implementation
//!    does, in every pairing of "both", "1.2 only" and "1.3 only".
//! 2. **An active attacker** between the two, with rustls as the honest peer:
//!    strip TLS 1.3 from a hello and the 1.3-capable server answers in 1.2
//!    with the `DOWNGRD` sentinel, which the client that offered 1.3 must
//!    refuse. rustls computes the sentinel in one direction and checks it in
//!    the other, so each half of this implementation is tested against an
//!    independent reading of RFC 8446 §4.1.3.
//! 3. **The inverse**: a 1.2-only client of a 1.3-capable server sees the
//!    sentinel and must carry on.
//! 4. **`TLS_FALLBACK_SCSV`** and the other refusals the choice makes.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{
    record_length, AlertDescription, CipherSuite, ClientConfig, ClientError, Incoming,
};
use rusty_tls::handrolled::client12::{CipherSuite12, Incoming12};
use rusty_tls::handrolled::handshake::{extension, messages, Extension, HandshakeType};
use rusty_tls::handrolled::handshake12::{message, ClientHello12, ServerHello12};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, Established, ServerConfigBoth, ServerHandshakeBoth,
    Version,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::record::ContentType;
use rusty_tls::handrolled::server::{ServerConfig, ServerError};
use rusty_tls::handrolled::server12::ServerConfig12;
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

/// A CA, and a leaf it issued.
struct Pki {
    root_der: Vec<u8>,
    leaf_der: Vec<u8>,
    leaf_pkcs8: Vec<u8>,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

/// As [`pki`], with a hook to adjust the leaf's parameters (to expire it).
fn pki(name: &str) -> Pki {
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

    let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("leaf key");
    let mut leaf_params = CertificateParams::new(vec![name.to_string()]).expect("leaf params");
    dated(&mut leaf_params);
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

fn signing_key(pki: &Pki) -> SigningKey {
    SigningKey::ecdsa_p256(&pki.leaf_pkcs8).expect("the leaf key loads")
}

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

fn record(typ: ContentType, fragment: &[u8]) -> Vec<u8> {
    let mut out = vec![typ.as_u8(), 3, 3];
    out.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
    out.extend_from_slice(fragment);
    out
}

/// RFC 8446 §4.1.3, written out so a change to the library's constant cannot
/// change what these tests demand.
const DOWNGRADE_SENTINEL: [u8; 8] = *b"DOWNGRD\x01";

const ALL_GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

/// A server's material, owned in one place.
struct Material {
    pki: Pki,
    certificates: Vec<Vec<u8>>,
    key: SigningKey,
}

impl Material {
    fn new() -> Self {
        let pki = pki(SERVER);
        let certificates = vec![pki.leaf_der.clone(), pki.root_der.clone()];
        let key = signing_key(&pki);
        Self {
            pki,
            certificates,
            key,
        }
    }

    fn server_13(&self) -> ServerConfig<'_> {
        ServerConfig {
            certificates: &self.certificates,
            key: &self.key,
            cipher_suites: CipherSuite::SUPPORTED,
            groups: ALL_GROUPS,
            client_auth: None,
            tickets: None,
        }
    }

    fn server_12(&self) -> ServerConfig12<'_> {
        ServerConfig12 {
            certificates: &self.certificates,
            key: &self.key,
            cipher_suites: CipherSuite12::SUPPORTED,
            groups: ALL_GROUPS,
            client_auth: None,
        }
    }
}

fn client_13<'a>(anchors: &'a [TrustAnchor<'a>]) -> ClientConfig<'a> {
    ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors,
        path: options(),
        groups: ALL_GROUPS,
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    }
}

// ---------------------------------------------------------------------------
// rustls peers
// ---------------------------------------------------------------------------

type Versions = &'static [&'static rustls::SupportedProtocolVersion];
const ONLY_12: Versions = &[&rustls::version::TLS12];
const ONLY_13: Versions = &[&rustls::version::TLS13];
const BOTH: Versions = &[&rustls::version::TLS13, &rustls::version::TLS12];

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn rustls_client(root_der: &[u8], versions: Versions) -> rustls::ClientConnection {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(root_der.to_vec()))
        .expect("rustls accepts the root");
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(versions)
        .expect("versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from(SERVER).expect("name"),
    )
    .expect("client connection")
}

fn rustls_server(material: &Material, versions: Versions) -> rustls::ServerConnection {
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(versions)
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(material.pki.chain.clone(), material.pki.key.clone_key())
        .expect("server config");
    rustls::ServerConnection::new(Arc::new(config)).expect("server connection")
}

/// A rustls connection, either end, driven the same way.
trait Peer {
    fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Option<String>);
}

macro_rules! impl_peer {
    ($ty:ty) => {
        impl Peer for $ty {
            fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Option<String>) {
                let mut refusal = None;
                if !input.is_empty() {
                    let mut cursor = std::io::Cursor::new(input);
                    while self.read_tls(&mut cursor).unwrap_or(0) > 0 {
                        if let Err(err) = self.process_new_packets() {
                            refusal = Some(err.to_string());
                            break;
                        }
                    }
                }
                if refusal.is_none() {
                    if let Err(err) = self.process_new_packets() {
                        refusal = Some(err.to_string());
                    }
                }
                let mut out = Vec::new();
                while self.wants_write() {
                    self.write_tls(&mut out).expect("write_tls");
                }
                (out, refusal)
            }
        }
    };
}
impl_peer!(rustls::ClientConnection);
impl_peer!(rustls::ServerConnection);

// ---------------------------------------------------------------------------
// Helpers on the established connection
// ---------------------------------------------------------------------------

fn send(connection: &mut Established, data: &[u8]) -> Vec<u8> {
    match connection {
        Established::Tls13(c) => c.write(data).expect("write"),
        Established::Tls12(c) => c.write(data).expect("write"),
        _ => unreachable!("only two versions"),
    }
}

/// What a record carried, if it was application data.
fn receive(connection: &mut Established, record: &[u8]) -> Vec<u8> {
    match connection {
        Established::Tls13(c) => match c.read(record).expect("read") {
            Incoming::Application(data) => data,
            _ => Vec::new(),
        },
        Established::Tls12(c) => match c.read(record).expect("read") {
            Incoming12::Application(data) => data,
            _ => Vec::new(),
        },
        _ => unreachable!("only two versions"),
    }
}

fn expect_version(connection: &Established, version: Version) {
    assert_eq!(connection.version(), version);
}

/// The `random` of the first ServerHello in a stream of records.
fn server_hello_random(records: &[Vec<u8>]) -> Vec<u8> {
    let fragment = &records[0][5..];
    let parsed = messages(fragment).expect("parses");
    assert_eq!(parsed[0].typ, HandshakeType::ServerHello);
    ServerHello12::parse(parsed[0].body)
        .expect("a ServerHello")
        .random
        .to_vec()
}

/// A ClientHello record with its `supported_versions` replaced, as an attacker
/// who cannot break the connection but can rewrite its first message would.
fn rewrite_versions(record: &[u8], versions: &[u8]) -> Vec<u8> {
    let parsed = messages(&record[5..]).expect("a hello");
    let hello = ClientHello12::parse(parsed[0].body).expect("parses");
    let mut owned: Vec<(u16, Vec<u8>)> = hello
        .extensions
        .iter()
        .map(|e| (e.typ, e.data.to_vec()))
        .collect();
    for (typ, data) in &mut owned {
        if *typ == extension::SUPPORTED_VERSIONS {
            let mut list = vec![versions.len() as u8];
            list.extend_from_slice(versions);
            *data = list;
        }
    }
    reencode(&hello, &owned, &hello.cipher_suites)
}

fn reencode(hello: &ClientHello12<'_>, extensions: &[(u16, Vec<u8>)], suites: &[u16]) -> Vec<u8> {
    let extensions = extensions
        .iter()
        .map(|(typ, data)| Extension { typ: *typ, data })
        .collect();
    let body = ClientHello12 {
        version: hello.version,
        random: hello.random,
        session_id: hello.session_id,
        cipher_suites: suites.to_vec(),
        compression: hello.compression,
        extensions,
    }
    .encode();
    record(
        ContentType::Handshake,
        &message(HandshakeType::ClientHello, &body),
    )
}

fn with_suite(record: &[u8], suite: u16) -> Vec<u8> {
    let parsed = messages(&record[5..]).expect("a hello");
    let hello = ClientHello12::parse(parsed[0].body).expect("parses");
    let owned: Vec<(u16, Vec<u8>)> = hello
        .extensions
        .iter()
        .map(|e| (e.typ, e.data.to_vec()))
        .collect();
    let mut suites = hello.cipher_suites.clone();
    suites.push(suite);
    reencode(&hello, &owned, &suites)
}

/// Split every plaintext handshake record in `records` into pieces of `size`.
fn reframe(records: Vec<Vec<u8>>, size: usize) -> Vec<Vec<u8>> {
    records
        .into_iter()
        .flat_map(|r| {
            if r[0] == ContentType::Handshake.as_u8() {
                r[5..]
                    .chunks(size)
                    .map(|piece| record(ContentType::Handshake, piece))
                    .collect()
            } else {
                vec![r]
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Our server, a rustls client
// ---------------------------------------------------------------------------

struct ServerRun {
    result: Result<Established, ServerError>,
    /// The sentinel-bearing field of the first ServerHello.
    random: Option<Vec<u8>>,
    client_refusal: Option<String>,
    client: rustls::ClientConnection,
}

/// Drive our two-version server against a rustls client. `tamper` rewrites the
/// client's first record on its way; `frame` re-frames the server's first
/// flight.
fn serve_rustls_client(
    material: &Material,
    versions: Versions,
    tamper: impl FnOnce(&[u8]) -> Vec<u8>,
    frame: usize,
) -> ServerRun {
    let (tls13, tls12) = (material.server_13(), material.server_12());
    let config = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let mut server = ServerHandshakeBoth::new(&config);
    let mut client = rustls_client(&material.pki.root_der, versions);

    let mut to_server = client.feed(&[]).0;
    let mut tamper = Some(tamper);
    let mut random = None;
    let mut refusal = None;
    for _ in 0..16 {
        let mut records = take_records(&mut to_server);
        if let Some(tamper) = tamper.take() {
            records[0] = tamper(&records[0]);
        }
        let mut to_client = Vec::new();
        for record in records {
            match server.read_record(&record) {
                Ok(reply) => to_client.extend_from_slice(&reply),
                Err(error) => {
                    return ServerRun {
                        result: Err(error),
                        random,
                        client_refusal: refusal,
                        client,
                    }
                }
            }
        }
        if server.is_finished() {
            let (_, r) = client.feed(&to_client);
            return ServerRun {
                result: Ok(server.into_connection().expect("finished")),
                random,
                client_refusal: r.or(refusal),
                client,
            };
        }
        let mut flight = take_records(&mut to_client.clone());
        if random.is_none() && !flight.is_empty() {
            random = Some(server_hello_random(&flight));
        }
        if frame > 0 && !flight.is_empty() {
            flight = reframe(flight, frame);
        }
        let joined: Vec<u8> = flight.concat();
        let (bytes, r) = client.feed(&joined);
        refusal = r.or(refusal);
        if refusal.is_some() || bytes.is_empty() {
            // The client said no (or has nothing more to say); let the server
            // see whatever alert it sent.
            for record in take_records(&mut bytes.clone()) {
                let _ = server.read_record(&record);
            }
            return ServerRun {
                result: Err(ServerError::Failed),
                random,
                client_refusal: refusal,
                client,
            };
        }
        to_server = bytes;
    }
    panic!("the handshake neither finished nor failed");
}

fn untouched(record: &[u8]) -> Vec<u8> {
    record.to_vec()
}

/// Carry data both ways between our connection and a rustls client.
fn exchange_with_client(run: &mut ServerRun) {
    use std::io::{Read, Write};
    let connection = run.result.as_mut().expect("established");

    run.client
        .writer()
        .write_all(b"to the server")
        .expect("write");
    let (bytes, _) = run.client.feed(&[]);
    let mut got = Vec::new();
    for r in take_records(&mut bytes.clone()) {
        got.extend(receive(connection, &r));
    }
    assert_eq!(got, b"to the server");

    let mut wire = send(connection, b"to the client");
    for r in take_records(&mut wire) {
        let (_, refusal) = run.client.feed(&r);
        assert_eq!(refusal, None);
    }
    let mut buf = [0u8; 64];
    let n = run.client.reader().read(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"to the client");
}

#[test]
fn the_server_agrees_with_rustls_on_the_version_in_every_pairing() {
    let material = Material::new();
    for (versions, want) in [
        (BOTH, Version::Tls13),
        (ONLY_13, Version::Tls13),
        (ONLY_12, Version::Tls12),
    ] {
        let mut run = serve_rustls_client(&material, versions, untouched, 0);
        assert_eq!(run.client_refusal, None, "{want:?}");
        expect_version(run.result.as_ref().expect("established"), want);
        exchange_with_client(&mut run);
    }
}

#[test]
fn a_hello_is_negotiated_however_it_is_framed() {
    // The server's choice is made on a whole hello, whatever the transport did
    // to it. (The server's own flight is re-framed too, for the client's sake.)
    let material = Material::new();
    for versions in [BOTH, ONLY_12] {
        for size in [1usize, 3, 40] {
            let (tls13, tls12) = (material.server_13(), material.server_12());
            let config = ServerConfigBoth {
                tls13: &tls13,
                tls12: &tls12,
            };
            let mut client = rustls_client(&material.pki.root_der, versions);
            let hello = client.feed(&[]).0;
            let mut server = ServerHandshakeBoth::new(&config);
            let mut reply = Vec::new();
            for r in reframe(take_records(&mut hello.clone()), size) {
                reply.extend(server.read_record(&r).expect("a piece is accepted"));
            }
            assert!(!reply.is_empty());
            let want = if versions.len() == 2 {
                Version::Tls13
            } else {
                Version::Tls12
            };
            assert_eq!(server.version(), Some(want), "{size}");
        }
    }
}

// ---------------------------------------------------------------------------
// Our client, a rustls server
// ---------------------------------------------------------------------------

struct ClientRun {
    result: Result<Established, ClientError>,
    server: rustls::ServerConnection,
    /// What the server sent after the handshake (session tickets, in 1.3).
    /// Each record advances the connection's sequence number, so it has to be
    /// read before anything that follows will decrypt.
    pending: Vec<u8>,
}

fn connect_to_rustls_server(
    material: &Material,
    versions: Versions,
    tamper: impl FnOnce(&[u8]) -> Vec<u8>,
    frame: usize,
) -> ClientRun {
    let anchors = [anchor(&material.pki.root_der)];
    let config = ClientConfigBoth::new(client_13(&anchors), CipherSuite12::SUPPORTED);
    let mut server = rustls_server(material, versions);

    let (mut client, hello) = match ClientHandshakeBoth::start(&config) {
        Ok(started) => started,
        Err(error) => panic!("start: {error}"),
    };
    let mut to_server = tamper(&hello);
    for round in 0..16 {
        let (bytes, _) = server.feed(&to_server);
        to_server.clear();
        let mut flight = take_records(&mut bytes.clone());
        if round == 0 && frame > 0 {
            flight = reframe(flight, frame);
        }
        for r in flight {
            match client.read_record(&r) {
                Ok(reply) => to_server.extend_from_slice(&reply),
                Err(error) => {
                    return ClientRun {
                        result: Err(error),
                        server,
                        pending: Vec::new(),
                    }
                }
            }
        }
        if client.is_finished() {
            let connection = client.into_connection().expect("finished");
            let (pending, _) = server.feed(&to_server);
            return ClientRun {
                result: Ok(connection),
                server,
                pending,
            };
        }
        if to_server.is_empty() {
            break;
        }
    }
    panic!("the handshake neither finished nor failed");
}

fn exchange_with_server(run: &mut ClientRun) {
    use std::io::{Read, Write};
    let connection = run.result.as_mut().expect("established");
    for r in take_records(&mut run.pending) {
        assert!(receive(connection, &r).is_empty());
    }

    let mut wire = send(connection, b"to the server");
    for r in take_records(&mut wire) {
        let (_, refusal) = run.server.feed(&r);
        assert_eq!(refusal, None);
    }
    let mut buf = [0u8; 64];
    let n = run.server.reader().read(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"to the server");

    run.server
        .writer()
        .write_all(b"to the client")
        .expect("write");
    let (bytes, _) = run.server.feed(&[]);
    let mut got = Vec::new();
    for r in take_records(&mut bytes.clone()) {
        got.extend(receive(connection, &r));
    }
    assert_eq!(got, b"to the client");
}

#[test]
fn the_client_agrees_with_rustls_on_the_version_in_every_pairing() {
    let material = Material::new();
    for (versions, want) in [
        (BOTH, Version::Tls13),
        (ONLY_13, Version::Tls13),
        // A TLS 1.2-only rustls server writes no sentinel (it cannot speak
        // 1.3), and a client that offered both must accept it.
        (ONLY_12, Version::Tls12),
    ] {
        let mut run = connect_to_rustls_server(&material, versions, untouched, 0);
        expect_version(run.result.as_ref().expect("established"), want);
        exchange_with_server(&mut run);
    }
}

#[test]
fn a_server_hello_is_negotiated_however_it_is_framed() {
    let material = Material::new();
    for versions in [BOTH, ONLY_12] {
        for size in [1usize, 2, 7, 33] {
            let mut run = connect_to_rustls_server(&material, versions, untouched, size);
            assert!(
                run.result.is_ok(),
                "{size}: {:?}",
                run.result.as_ref().err()
            );
            exchange_with_server(&mut run);
        }
    }
}

// ---------------------------------------------------------------------------
// Downgrade
// ---------------------------------------------------------------------------

/// An attacker removes TLS 1.3 from the client's hello. The 1.3-capable server
/// answers in 1.2, and must say so in `random`; the client that offered 1.3
/// must refuse. Tested in both directions, with rustls as the honest end.
#[test]
fn a_stripped_hello_makes_our_server_signal_and_a_rustls_client_refuse() {
    let material = Material::new();
    let strip = |r: &[u8]| rewrite_versions(r, &[0x03, 0x03]);

    let run = serve_rustls_client(&material, BOTH, strip, 0);
    let random = run.random.expect("the server answered");
    assert_eq!(
        random[24..],
        DOWNGRADE_SENTINEL,
        "the server did not tell the client it was downgraded"
    );
    let refusal = run.client_refusal.expect("rustls accepted a downgrade");
    assert!(
        refusal.to_lowercase().contains("downgrade"),
        "refused, but not for the downgrade: {refusal}"
    );
    assert!(run.result.is_err());

    // The control: the same client, hello untouched, completes in 1.3 with no
    // sentinel anywhere.
    let run = serve_rustls_client(&material, BOTH, untouched, 0);
    assert_ne!(run.random.expect("random")[24..], DOWNGRADE_SENTINEL);
    expect_version(&run.result.expect("completes"), Version::Tls13);
}

#[test]
fn a_stripped_hello_is_refused_by_our_client_when_the_server_signals() {
    let material = Material::new();
    let strip = |r: &[u8]| rewrite_versions(r, &[0x03, 0x03]);

    let run = connect_to_rustls_server(&material, BOTH, strip, 0);
    assert!(
        matches!(run.result, Err(ClientError::DowngradeDetected)),
        "{:?}",
        run.result.err()
    );
}

#[test]
fn a_stripped_hello_against_a_server_that_cannot_signal_fails_at_the_finished() {
    // A TLS 1.2-only server has no sentinel to send, so this is the case the
    // sentinel cannot catch. The transcript does: the server saw a different
    // hello than the client sent, and the Finished messages disagree.
    let material = Material::new();
    let strip = |r: &[u8]| rewrite_versions(r, &[0x03, 0x03]);

    let run = connect_to_rustls_server(&material, ONLY_12, strip, 0);
    let error = run.result.expect_err("a rewritten hello cannot complete");
    assert!(
        !matches!(error, ClientError::DowngradeDetected),
        "{error:?}"
    );
}

#[test]
fn the_sentinel_is_only_written_when_this_server_could_have_spoken_1_3() {
    // The same 1.2 server, standing alone, tells a 1.2-only client nothing it
    // would have to ignore.
    let material = Material::new();
    let tls12 = material.server_12();
    let anchors = [anchor(&material.pki.root_der)];
    let config = rusty_tls::handrolled::client12::ClientConfig12 {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: ALL_GROUPS,
        cipher_suites: CipherSuite12::SUPPORTED,
    };
    let (_, hello) =
        rusty_tls::handrolled::client12::ClientHandshake12::start(&config).expect("hello");
    let mut server =
        rusty_tls::handrolled::server12::ServerHandshake12::new(&tls12).expect("server");
    let mut flight = server.read_record(&hello).expect("flight");
    let random = server_hello_random(&take_records(&mut flight));
    assert_ne!(random[24..], DOWNGRADE_SENTINEL);
}

#[test]
fn a_one_two_only_client_is_not_alarmed_by_the_sentinel() {
    // Our two-version server answers a TLS 1.2-only rustls client in 1.2 with
    // the sentinel in place (see `the_server_agrees_with_rustls...`). That
    // client has to complete: a 1.2-only client of a 1.3-capable server is
    // told the truth, and it is not under attack.
    let material = Material::new();
    let run = serve_rustls_client(&material, ONLY_12, untouched, 0);
    assert_eq!(run.random.expect("random")[24..], DOWNGRADE_SENTINEL);
    assert_eq!(run.client_refusal, None);
    expect_version(&run.result.expect("completes"), Version::Tls12);
}

// ---------------------------------------------------------------------------
// TLS_FALLBACK_SCSV and the other refusals the choice makes
// ---------------------------------------------------------------------------

fn first_flight_of(
    material: &Material,
    hello: &[u8],
) -> Result<Vec<u8>, (ServerError, Option<Vec<u8>>)> {
    let (tls13, tls12) = (material.server_13(), material.server_12());
    let config = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let mut server = ServerHandshakeBoth::new(&config);
    server.read_record(hello).map_err(|error| {
        let alert = server.alert_record(&error);
        assert_eq!(server.read_record(hello), Err(ServerError::Failed));
        (error, alert)
    })
}

/// A hello from a client that speaks only TLS 1.2.
fn hello_12(material: &Material) -> Vec<u8> {
    let anchors = [anchor(&material.pki.root_der)];
    let config = rusty_tls::handrolled::client12::ClientConfig12 {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: ALL_GROUPS,
        cipher_suites: CipherSuite12::SUPPORTED,
    };
    rusty_tls::handrolled::client12::ClientHandshake12::start(&config)
        .expect("hello")
        .1
}

/// A hello from a client that offers both.
fn hello_both(material: &Material) -> Vec<u8> {
    let anchors = [anchor(&material.pki.root_der)];
    let config = ClientConfigBoth::new(client_13(&anchors), CipherSuite12::SUPPORTED);
    ClientHandshakeBoth::start(&config).expect("hello").1
}

const FALLBACK_SCSV: u16 = 0x5600;

#[test]
fn a_fallback_retry_that_offers_less_than_the_server_speaks_is_refused() {
    let material = Material::new();
    let plain = hello_12(&material);
    assert!(first_flight_of(&material, &plain).is_ok(), "the control");

    let (error, alert) = first_flight_of(&material, &with_suite(&plain, FALLBACK_SCSV))
        .expect_err("a fallback to 1.2 from a server that speaks 1.3");
    assert_eq!(error, ServerError::InappropriateFallback);
    assert_eq!(
        error.alert(),
        Some(AlertDescription::INAPPROPRIATE_FALLBACK)
    );
    assert_eq!(alert.expect("an alert")[5..7], [2, 86]);

    // A client that offers 1.3 has not fallen back from anything.
    let both = with_suite(&hello_both(&material), FALLBACK_SCSV);
    assert!(first_flight_of(&material, &both).is_ok());
}

#[test]
fn the_choice_refuses_what_it_cannot_read_or_cannot_serve() {
    let material = Material::new();
    let hello = hello_12(&material);

    let versions = |list: &[u8]| rewrite_versions(&hello_both(&material), list);
    // Offering neither 1.3 nor 1.2.
    for list in [
        &[0x03, 0x02][..],
        &[0x03, 0x02, 0x03, 0x01],
        &[0x03, 0x04, 0x03, 0x03],
    ] {
        let result = first_flight_of(&material, &versions(list));
        if list.contains(&0x04) {
            assert!(result.is_ok(), "{list:?}");
        } else {
            let (error, alert) = result.expect_err("neither version offered");
            assert!(matches!(error, ServerError::NotTls12(_)), "{error:?}");
            assert_eq!(alert.expect("alert")[5..7], [2, 70]);
        }
    }
    // Only 1.2 in a list, and only 1.3.
    assert_eq!(
        server_version_for(&material, &versions(&[0x03, 0x03])),
        Version::Tls12
    );
    assert_eq!(
        server_version_for(&material, &versions(&[0x03, 0x04])),
        Version::Tls13
    );

    // client_version below 1.2 and no extension at all.
    let mut old = hello.clone();
    old[10] = 0x02; // the hello's client_version, minor octet
    let (error, _) = first_flight_of(&material, &old).expect_err("TLS 1.1");
    assert_eq!(error, ServerError::NotTls12(0x0302));

    // A malformed list is malformed, not "no".
    let (tls13, tls12) = (material.server_13(), material.server_12());
    let _ = (&tls13, &tls12);
    for bad in [&[][..], &[0x03][..]] {
        let mut malformed = hello_both(&material);
        let parsed = messages(&malformed[5..]).expect("hello").to_vec();
        let body = parsed[0].body;
        let hello = ClientHello12::parse(body).expect("parses");
        let owned: Vec<(u16, Vec<u8>)> = hello
            .extensions
            .iter()
            .map(|e| {
                if e.typ == extension::SUPPORTED_VERSIONS {
                    (e.typ, bad.to_vec())
                } else {
                    (e.typ, e.data.to_vec())
                }
            })
            .collect();
        malformed = reencode(&hello, &owned, &hello.cipher_suites);
        let (error, alert) = first_flight_of(&material, &malformed).expect_err("malformed");
        assert!(
            matches!(error, ServerError::Handshake(_)),
            "{bad:?}: {error:?}"
        );
        assert_eq!(alert.expect("alert")[5..7], [2, 50]);
    }

    // Not a ClientHello at all.
    let (error, _) =
        first_flight_of(&material, &record(ContentType::ApplicationData, b"hi")).expect_err("data");
    assert!(
        matches!(error, ServerError::UnexpectedContentType(_)),
        "{error:?}"
    );
    let (error, _) = first_flight_of(
        &material,
        &record(
            ContentType::Handshake,
            &message(HandshakeType::Finished, &[0; 12]),
        ),
    )
    .expect_err("a Finished");
    assert!(
        matches!(error, ServerError::UnexpectedMessage { .. }),
        "{error:?}"
    );
}

fn server_version_for(material: &Material, hello: &[u8]) -> Version {
    let (tls13, tls12) = (material.server_13(), material.server_12());
    let config = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let mut server = ServerHandshakeBoth::new(&config);
    server.read_record(hello).expect("accepted");
    server.version().expect("chosen")
}

#[test]
fn an_unbounded_first_message_is_cut_off_on_both_sides() {
    let material = Material::new();
    let (tls13, tls12) = (material.server_13(), material.server_12());
    let config = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let mut server = ServerHandshakeBoth::new(&config);
    let mut first = vec![1, 0xff, 0xff, 0xff];
    first.resize(16_384, 0);
    let mut sent = 0;
    let error = loop {
        let fragment = if sent == 0 {
            first.clone()
        } else {
            vec![0; 16_384]
        };
        match server.read_record(&record(ContentType::Handshake, &fragment)) {
            Ok(_) => sent += 1,
            Err(error) => break error,
        }
        assert!(sent < 16, "the server buffered without limit");
    };
    assert_eq!(error, ServerError::HandshakeTooLarge);

    let anchors = [anchor(&material.pki.root_der)];
    let config = ClientConfigBoth::new(client_13(&anchors), CipherSuite12::SUPPORTED);
    let (mut client, _) = ClientHandshakeBoth::start(&config).expect("start");
    let mut first = vec![2, 0xff, 0xff, 0xff];
    first.resize(16_384, 0);
    let mut sent = 0;
    let error = loop {
        let fragment = if sent == 0 {
            first.clone()
        } else {
            vec![0; 16_384]
        };
        match client.read_record(&record(ContentType::Handshake, &fragment)) {
            Ok(_) => sent += 1,
            Err(error) => break error,
        }
        assert!(sent < 16, "the client buffered without limit");
    };
    assert!(matches!(error, ClientError::HandshakeTooLarge), "{error:?}");
    assert!(matches!(client.read_record(&[]), Err(ClientError::Failed)));
}

#[test]
fn a_server_alert_before_the_server_hello_is_reported_by_name() {
    // A server too old for both versions says so in the clear; whichever
    // machine reads it must surface that, not "unexpected content type".
    let material = Material::new();
    let anchors = [anchor(&material.pki.root_der)];
    let config = ClientConfigBoth::new(client_13(&anchors), CipherSuite12::SUPPORTED);
    let (mut client, _) = ClientHandshakeBoth::start(&config).expect("start");
    let error = client
        .read_record(&record(ContentType::Alert, &[2, 70]))
        .expect_err("an alert");
    assert!(
        matches!(error, ClientError::PeerAlert(a) if a.description == AlertDescription::PROTOCOL_VERSION),
        "{error:?}"
    );
}

#[test]
fn a_record_that_ends_inside_the_next_message_does_not_confuse_the_choice() {
    // The hello, then the first two octets of whatever comes next, in one
    // record. The version is judged by the complete hello alone; the stray
    // octets are the chosen machine's business.
    let material = Material::new();
    for (hello, want) in [
        (hello_12(&material), Version::Tls12),
        (hello_both(&material), Version::Tls13),
    ] {
        let mut fragment = hello[5..].to_vec();
        fragment.extend_from_slice(&[20, 0]);
        let straddling = record(ContentType::Handshake, &fragment);
        assert_eq!(server_version_for(&material, &straddling), want);
    }
}

#[test]
fn the_combined_hello_offers_both_and_the_plain_one_offers_only_1_3() {
    let material = Material::new();
    let anchors = [anchor(&material.pki.root_der)];

    let parse = |record: &[u8]| {
        let parsed = messages(&record[5..]).expect("a hello");
        let body = parsed[0].body.to_vec();
        let hello = ClientHello12::parse(&body).expect("parses");
        let get = |typ| {
            hello
                .extensions
                .iter()
                .find(|e| e.typ == typ)
                .map(|e| e.data.to_vec())
        };
        (
            hello.cipher_suites.clone(),
            get(extension::SUPPORTED_VERSIONS),
            get(extension::SIGNATURE_ALGORITHMS),
            get(extension::EXTENDED_MASTER_SECRET),
            get(extension::RENEGOTIATION_INFO),
            get(extension::EC_POINT_FORMATS),
        )
    };

    let (suites, versions, schemes, ems, reneg, formats) = parse(&hello_both(&material));
    assert_eq!(versions, Some(vec![4, 3, 4, 3, 3]), "1.3 first, then 1.2");
    for suite in CipherSuite::SUPPORTED {
        assert!(suites.contains(&suite.0));
    }
    for suite in CipherSuite12::SUPPORTED {
        assert!(suites.contains(&suite.0));
    }
    // The first suite is TLS 1.3's: the server's preference order is its own,
    // but a client lists the version it prefers first.
    assert_eq!(suites[0], CipherSuite::SUPPORTED[0].0);
    let schemes = schemes.expect("signature_algorithms");
    assert!(
        schemes.chunks(2).any(|s| s == [0x04, 0x01]),
        "the PKCS#1 schemes a TLS 1.2 RSA server may sign with"
    );
    assert_eq!(ems, Some(vec![]));
    assert_eq!(reneg, Some(vec![0]));
    assert_eq!(formats, Some(vec![1, 0]));

    // A client that never asked for TLS 1.2 says nothing about it.
    let config = client_13(&anchors);
    let (_, plain) = rusty_tls::handrolled::client::ClientHandshake::start(&config).expect("hello");
    let (suites, versions, schemes, ems, reneg, formats) = parse(&plain);
    assert_eq!(versions, Some(vec![2, 3, 4]));
    assert!(CipherSuite12::SUPPORTED
        .iter()
        .all(|s| !suites.contains(&s.0)));
    assert!(!schemes
        .expect("signature_algorithms")
        .chunks(2)
        .any(|s| s == [0x04, 0x01]));
    assert_eq!((ems, reneg, formats), (None, None, None));
}

#[test]
fn a_tls12_only_client_completes_against_the_two_version_server_despite_the_sentinel() {
    // This crate's own 1.2-only client, which does not check the sentinel (it
    // offered nothing to be downgraded from), against the server that writes
    // it: the pairing that would break if the check were applied too widely.
    let material = Material::new();
    let (tls13, tls12) = (material.server_13(), material.server_12());
    let config = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let anchors = [anchor(&material.pki.root_der)];
    let client_config = rusty_tls::handrolled::client12::ClientConfig12 {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: ALL_GROUPS,
        cipher_suites: CipherSuite12::SUPPORTED,
    };
    let (mut client, hello) =
        rusty_tls::handrolled::client12::ClientHandshake12::start(&client_config).expect("hello");
    let mut server = ServerHandshakeBoth::new(&config);

    let mut to_client = server.read_record(&hello).expect("flight");
    let mut to_server = Vec::new();
    for r in take_records(&mut to_client) {
        to_server.extend(
            client
                .read_record(&r)
                .expect("the sentinel is not an error here"),
        );
    }
    let mut to_client = Vec::new();
    for r in take_records(&mut to_server) {
        to_client.extend(server.read_record(&r).expect("client flight"));
    }
    for r in take_records(&mut to_client) {
        client.read_record(&r).expect("server finished");
    }
    assert!(client.is_finished() && server.is_finished());
    expect_version(
        &server.into_connection().expect("connection"),
        Version::Tls12,
    );
}

#[test]
fn a_retried_hello_still_offers_what_the_first_did() {
    // A server that wants a group the first hello had no share for sends a
    // HelloRetryRequest, and the second hello may change only a few things
    // (RFC 8446 §4.1.2). Which versions and suites are offered is not among
    // them, and must survive the retry.
    let material = Material::new();
    let anchors = [anchor(&material.pki.root_der)];
    let config = ClientConfigBoth::new(client_13(&anchors), CipherSuite12::SUPPORTED);

    let mut provider = rustls::crypto::ring::default_provider();
    provider.kx_groups = vec![rustls::crypto::ring::kx_group::SECP256R1];
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(BOTH)
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(material.pki.chain.clone(), material.pki.key.clone_key())
        .expect("server config");
    let mut server = rustls::ServerConnection::new(Arc::new(server_config)).expect("server");

    let (mut client, first) = ClientHandshakeBoth::start(&config).expect("start");
    let (bytes, refusal) = server.feed(&first);
    assert_eq!(refusal, None);
    let mut reply = Vec::new();
    for r in take_records(&mut bytes.clone()) {
        reply.extend(
            client
                .read_record(&r)
                .expect("the retry request is accepted"),
        );
    }
    let second = take_records(&mut reply.clone())
        .into_iter()
        .find(|r| r[0] == 22)
        .expect("a second hello");

    let hello_of = |record: &[u8]| {
        let parsed = messages(&record[5..]).expect("hello");
        let body = parsed[0].body.to_vec();
        let hello = ClientHello12::parse(&body).expect("parses");
        (
            hello.cipher_suites.clone(),
            hello.supported_versions().expect("versions"),
            hello.extensions.iter().map(|e| e.typ).collect::<Vec<_>>(),
        )
    };
    let (first_suites, first_versions, first_extensions) = hello_of(&first);
    let (suites, versions, extensions) = hello_of(&second);
    assert_eq!(suites, first_suites);
    assert_eq!(versions, first_versions);
    assert_eq!(extensions, first_extensions);

    // And it completes, in 1.3.
    let mut to_server = reply;
    for _ in 0..8 {
        let (bytes, _) = server.feed(&to_server);
        to_server.clear();
        for r in take_records(&mut bytes.clone()) {
            to_server.extend(client.read_record(&r).expect("handshake"));
        }
        if client.is_finished() {
            break;
        }
    }
    assert_eq!(client.version(), Some(Version::Tls13));
    assert!(client.is_finished());
}
