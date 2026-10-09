//! Keying material exporters — stage 13.
//!
//! RFC 5705 (TLS 1.2) and RFC 8446 section 7.5 (TLS 1.3). An exporter is only
//! worth anything if the other end computes the same bytes, so the oracle is a
//! real peer: rustls exports from its side of the same connection and the two
//! must agree, in each direction, for each version, and with each shape of
//! context (absent, empty, present). OpenSSL's `-keymatexport` is the second
//! opinion in the interop suite.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{record_length, CipherSuite, ClientConfig};
use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::export::ExportError;
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, Established, ServerConfigBoth, ServerHandshakeBoth,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::ServerConfig;
use rusty_tls::handrolled::server12::ServerConfig12;
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::x509::Certificate;

const SERVER: &str = "export.example";

type Version = &'static rustls::SupportedProtocolVersion;
const VERSIONS: [(&str, Version); 2] = [
    ("TLS 1.2", &rustls::version::TLS12),
    ("TLS 1.3", &rustls::version::TLS13),
];

fn options() -> PathOptions {
    PathOptions {
        time: 1_800_000_000,
        max_path_length: 8,
        max_signature_checks: 64,
        required_eku: None,
    }
}

struct Pki {
    root_der: Vec<u8>,
    chain: Vec<Vec<u8>>,
    pkcs8: Vec<u8>,
}

fn pki() -> Pki {
    let dated = |params: &mut CertificateParams| {
        params.not_before = OffsetDateTime::from_unix_timestamp(1_577_836_800).expect("date");
        params.not_after = OffsetDateTime::from_unix_timestamp(1_893_456_000).expect("date");
    };
    let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    root_params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("export test root".to_string()),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");
    let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("leaf key");
    let mut leaf_params = CertificateParams::new(vec![SERVER.to_string()]).expect("params");
    dated(&mut leaf_params);
    let leaf = leaf_params
        .signed_by(&leaf_key, &root, &root_key)
        .expect("leaf");
    Pki {
        root_der: root.der().to_vec(),
        chain: vec![leaf.der().to_vec(), root.der().to_vec()],
        pkcs8: leaf_key.serialize_der(),
    }
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

fn pump(connection: &mut rustls::Connection, input: &[u8]) -> Vec<u8> {
    if !input.is_empty() {
        let mut cursor = std::io::Cursor::new(input);
        while connection.read_tls(&mut cursor).expect("read_tls") > 0 {
            connection.process_new_packets().expect("rustls accepts it");
        }
    }
    connection.process_new_packets().expect("rustls accepts it");
    let mut out = Vec::new();
    while connection.wants_write() {
        connection.write_tls(&mut out).expect("write_tls");
    }
    out
}

/// Our client against a rustls server pinned to `version`, both finished.
fn we_are_the_client(version: Version) -> (Established, rustls::Connection) {
    let pki = pki();
    let server_config = rustls::ServerConfig::builder_with_protocol_versions(&[version])
        .with_no_client_auth()
        .with_single_cert(
            pki.chain
                .iter()
                .cloned()
                .map(CertificateDer::from)
                .collect(),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pki.pkcs8.clone())),
        )
        .expect("server config");
    let mut server: rustls::Connection = rustls::ServerConnection::new(Arc::new(server_config))
        .expect("server")
        .into();
    let root = Certificate::parse(&pki.root_der).expect("root");
    let anchors = [TrustAnchor::from_certificate(&root)];
    let config = ClientConfigBoth::new(
        ClientConfig {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: &[NamedGroup::X25519],
            cipher_suites: CipherSuite::SUPPORTED,
            identity: None,
            resumption: None,
            alpn: &[],
        },
        CipherSuite12::SUPPORTED,
    );
    let (mut client, mut to_server) = ClientHandshakeBoth::start(&config).expect("start");
    for _ in 0..16 {
        let mut stream = pump(&mut server, &to_server);
        to_server.clear();
        for record in take_records(&mut stream) {
            to_server.extend(client.read_record(&record).expect("client"));
        }
        if client.is_finished() {
            // Let rustls see the client's Finished so its exporter is ready.
            pump(&mut server, &to_server);
            return (client.into_connection().expect("finished"), server);
        }
    }
    panic!("the handshake did not finish");
}

/// Our server against a rustls client pinned to `version`, both finished.
fn we_are_the_server(version: Version) -> (Established, rustls::Connection) {
    let pki = pki();
    let key = SigningKey::ecdsa_p256(&pki.pkcs8).expect("key");
    let tls13 = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: None,
        alpn: &[],
        sni: &[],
    };
    let tls12 = ServerConfig12 {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        alpn: &[],
        sni: &[],
    };
    let both = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(pki.root_der.clone()))
        .expect("root");
    let config = rustls::ClientConfig::builder_with_protocol_versions(&[version])
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut client: rustls::Connection = rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from(SERVER).expect("name"),
    )
    .expect("client")
    .into();

    let mut server = ServerHandshakeBoth::new(&both);
    let mut to_server = pump(&mut client, &[]);
    for _ in 0..16 {
        let mut reply = Vec::new();
        let mut stream = std::mem::take(&mut to_server);
        for record in take_records(&mut stream) {
            reply.extend(server.read_record(&record).expect("server"));
        }
        to_server = pump(&mut client, &reply);
        if server.is_finished() && to_server.is_empty() {
            break;
        }
    }
    (server.into_connection().expect("finished"), client)
}

/// The shapes of context an application can pass.
fn contexts() -> [Option<&'static [u8]>; 3] {
    [None, Some(b""), Some(b"application context")]
}

fn ours(connection: &Established, label: &[u8], context: Option<&[u8]>, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    connection
        .export_keying_material(label, context, &mut out)
        .expect("export");
    out
}

fn theirs(peer: &rustls::Connection, label: &[u8], context: Option<&[u8]>, len: usize) -> Vec<u8> {
    peer.export_keying_material(vec![0u8; len], label, context)
        .expect("rustls exports")
}

fn agree(name: &str, connection: &Established, peer: &rustls::Connection, version: &str) {
    for context in contexts() {
        for len in [1usize, 32, 100, 1024] {
            assert_eq!(
                ours(connection, b"EXPERIMENTAL label", context, len),
                theirs(peer, b"EXPERIMENTAL label", context, len),
                "{version} {name}: context {context:?}, {len} octets"
            );
        }
    }
}

#[test]
fn our_client_and_a_rustls_server_export_the_same_bytes() {
    for (name, version) in VERSIONS {
        let (connection, peer) = we_are_the_client(version);
        agree("client", &connection, &peer, name);
    }
}

#[test]
fn our_server_and_a_rustls_client_export_the_same_bytes() {
    for (name, version) in VERSIONS {
        let (connection, peer) = we_are_the_server(version);
        agree("server", &connection, &peer, name);
    }
}

#[test]
fn the_label_and_the_context_each_change_the_output() {
    for (name, version) in VERSIONS {
        let (connection, _) = we_are_the_client(version);
        let base = ours(&connection, b"label one", Some(b"ctx"), 32);
        assert_ne!(
            base,
            ours(&connection, b"label two", Some(b"ctx"), 32),
            "{name}"
        );
        assert_ne!(
            base,
            ours(&connection, b"label one", Some(b"other"), 32),
            "{name}"
        );
        // Deterministic for one connection.
        assert_eq!(
            base,
            ours(&connection, b"label one", Some(b"ctx"), 32),
            "{name}"
        );
    }
}

/// TLS 1.2 tells "no context" from "an empty context"; TLS 1.3 does not,
/// because both hash to the same value (RFC 5705 section 4, RFC 8446 7.5).
#[test]
fn only_tls12_distinguishes_no_context_from_an_empty_one() {
    let (tls12, _) = we_are_the_client(&rustls::version::TLS12);
    assert_ne!(
        ours(&tls12, b"label", None, 32),
        ours(&tls12, b"label", Some(b""), 32)
    );
    let (tls13, _) = we_are_the_client(&rustls::version::TLS13);
    assert_eq!(
        ours(&tls13, b"label", None, 32),
        ours(&tls13, b"label", Some(b""), 32)
    );
}

#[test]
fn what_cannot_be_exported_is_refused_and_says_why() {
    let (tls12, _) = we_are_the_client(&rustls::version::TLS12);
    let mut out = [0u8; 16];
    // RFC 5705 section 4: the labels TLS 1.2 itself uses are not available.
    for label in [
        &b"client finished"[..],
        b"server finished",
        b"master secret",
        b"key expansion",
    ] {
        assert_eq!(
            tls12.export_keying_material(label, None, &mut out),
            Err(ExportError::ReservedLabel),
            "{}",
            String::from_utf8_lossy(label)
        );
    }
    // A context that does not fit its two-octet length.
    let huge = vec![0u8; 65_536];
    assert_eq!(
        tls12.export_keying_material(b"label", Some(&huge), &mut out),
        Err(ExportError::ContextTooLong)
    );
    assert!(tls12
        .export_keying_material(b"label", Some(&huge[..65_535]), &mut out)
        .is_ok());

    let (tls13, _) = we_are_the_client(&rustls::version::TLS13);
    // The reserved TLS 1.2 labels mean nothing in 1.3, so they are not refused.
    assert!(tls13
        .export_keying_material(b"master secret", None, &mut out)
        .is_ok());
    // Not text; and too long for the one-octet label length.
    assert_eq!(
        tls13.export_keying_material(&[0xff, 0xfe], None, &mut out),
        Err(ExportError::BadLabel)
    );
    assert_eq!(
        tls13.export_keying_material(&[b'x'; 250], None, &mut out),
        Err(ExportError::BadLabel)
    );
    // More than HKDF can produce: 255 blocks of SHA-256 or SHA-384.
    let mut too_much = vec![0u8; 255 * 48 + 1];
    assert_eq!(
        tls13.export_keying_material(b"label", None, &mut too_much),
        Err(ExportError::TooLong)
    );
}
