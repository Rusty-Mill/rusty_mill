//! What a finished handshake can say about itself — stage 14.
//!
//! `key_exchange_group()`, `used_hello_retry_request()` and
//! `peer_signature_scheme()` report facts of the connection that the caller
//! cannot otherwise learn: which curve protected it, whether the handshake paid
//! for a retry, and what the peer signed with. A fact is only as good as the
//! other end's agreement, so rustls is the oracle where it can speak (the
//! group, and whether a retry happened) and the key type is where it cannot
//! (an ECDSA P-256 key signs with `ecdsa_secp256r1_sha256`, a P-384 key with
//! `ecdsa_secp384r1_sha384`, Ed25519 with `ed25519`).

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{record_length, CipherSuite, ClientConfig};
use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, Established, ServerConfigBoth, ServerHandshakeBoth,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::{ClientAuth, ServerConfig};
use rusty_tls::handrolled::server12::ServerConfig12;
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::verify::SignatureScheme;
use rusty_tls::handrolled::x509::Certificate;

const SERVER: &str = "negotiated.example";

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

#[derive(Clone, Copy, Debug)]
enum Kind {
    P256,
    P384,
    Ed25519,
}

impl Kind {
    fn algorithm(self) -> &'static rcgen::SignatureAlgorithm {
        match self {
            Self::P256 => &rcgen::PKCS_ECDSA_P256_SHA256,
            Self::P384 => &rcgen::PKCS_ECDSA_P384_SHA384,
            Self::Ed25519 => &rcgen::PKCS_ED25519,
        }
    }
    fn signing_key(self, pkcs8: &[u8]) -> SigningKey {
        match self {
            Self::P256 => SigningKey::ecdsa_p256(pkcs8),
            Self::P384 => SigningKey::ecdsa_p384(pkcs8),
            Self::Ed25519 => SigningKey::ed25519(pkcs8),
        }
        .expect("key")
    }
    fn scheme(self) -> SignatureScheme {
        match self {
            Self::P256 => SignatureScheme(0x0403),
            Self::P384 => SignatureScheme(0x0503),
            Self::Ed25519 => SignatureScheme(0x0807),
        }
    }
}

struct Pki {
    root_der: Vec<u8>,
    chain: Vec<Vec<u8>>,
    pkcs8: Vec<u8>,
}

fn pki(kind: Kind, name: &str) -> Pki {
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
        rcgen::DnValue::Utf8String(format!("negotiated test root for {name}")),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");
    let leaf_key = KeyPair::generate_for(kind.algorithm()).expect("leaf key");
    let mut leaf_params = CertificateParams::new(vec![name.to_string()]).expect("params");
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

/// A rustls provider with exactly these key exchange groups, in this order.
fn provider_with(
    groups: Vec<&'static dyn rustls::crypto::SupportedKxGroup>,
) -> Arc<rustls::crypto::CryptoProvider> {
    let mut provider = rustls::crypto::ring::default_provider();
    provider.kx_groups = groups;
    Arc::new(provider)
}

fn name_of(group: NamedGroup) -> rustls::NamedGroup {
    match group {
        NamedGroup::X25519 => rustls::NamedGroup::X25519,
        NamedGroup::SecP256R1 => rustls::NamedGroup::secp256r1,
        NamedGroup::SecP384R1 => rustls::NamedGroup::secp384r1,
        other => panic!("a group this test does not know: {other:?}"),
    }
}

/// What the two ends concluded about one handshake.
struct Facts {
    ours: Established,
    theirs: rustls::Connection,
}

/// Our client against a rustls server that supports exactly `server_groups`;
/// our client offers `client_groups` (its first is the share it sends).
fn client_side(
    version: Version,
    kind: Kind,
    client_groups: &[NamedGroup],
    server_groups: Vec<&'static dyn rustls::crypto::SupportedKxGroup>,
) -> Facts {
    let pki = pki(kind, SERVER);
    let config = rustls::ServerConfig::builder_with_provider(provider_with(server_groups))
        .with_protocol_versions(&[version])
        .expect("versions")
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
    let mut server: rustls::Connection = rustls::ServerConnection::new(Arc::new(config))
        .expect("server")
        .into();
    let root = Certificate::parse(&pki.root_der).expect("root");
    let anchors = [TrustAnchor::from_certificate(&root)];
    let config = ClientConfigBoth::new(
        ClientConfig {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: client_groups,
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
            pump(&mut server, &to_server);
            return Facts {
                ours: client.into_connection().expect("finished"),
                theirs: server,
            };
        }
    }
    panic!("the handshake did not finish");
}

/// Our server (optionally requiring a client certificate) against a rustls
/// client that supports exactly `client_groups`.
fn server_side(
    version: Version,
    server_kind: Kind,
    client_kind: Option<Kind>,
    server_groups: &[NamedGroup],
    client_groups: Vec<&'static dyn rustls::crypto::SupportedKxGroup>,
) -> Facts {
    let pki = pki(server_kind, SERVER);
    let key = server_kind.signing_key(&pki.pkcs8);
    let client_pki = client_kind.map(|kind| (kind, pki_for_client(kind)));
    let client_root = client_pki
        .as_ref()
        .map(|(_, p)| Certificate::parse(&p.root_der).expect("client root"));
    let anchors: Vec<TrustAnchor<'_>> = client_root
        .iter()
        .map(TrustAnchor::from_certificate)
        .collect();
    let auth = ClientAuth {
        anchors: &anchors,
        path: options(),
        required: true,
    };
    let client_auth = client_pki.is_some().then_some(&auth);
    let tls13 = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: server_groups,
        client_auth,
        tickets: None,
        alpn: &[],
        sni: &[],
    };
    let tls12 = ServerConfig12 {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: server_groups,
        client_auth,
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
    let builder = rustls::ClientConfig::builder_with_provider(provider_with(client_groups))
        .with_protocol_versions(&[version])
        .expect("versions")
        .with_root_certificates(roots);
    let config = match &client_pki {
        Some((_, p)) => builder
            .with_client_auth_cert(
                p.chain.iter().cloned().map(CertificateDer::from).collect(),
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(p.pkcs8.clone())),
            )
            .expect("client auth"),
        None => builder.with_no_client_auth(),
    };
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
    Facts {
        ours: server.into_connection().expect("finished"),
        theirs: client,
    }
}

fn pki_for_client(kind: Kind) -> Pki {
    pki(kind, "client.example")
}

#[test]
fn the_group_is_the_one_the_other_end_reports() {
    use rustls::crypto::ring::kx_group::{SECP256R1, SECP384R1, X25519};
    for (name, version) in VERSIONS {
        for (ours, theirs) in [
            (
                NamedGroup::X25519,
                X25519 as &'static dyn rustls::crypto::SupportedKxGroup,
            ),
            (NamedGroup::SecP256R1, SECP256R1),
            (NamedGroup::SecP384R1, SECP384R1),
        ] {
            let all = [
                NamedGroup::X25519,
                NamedGroup::SecP256R1,
                NamedGroup::SecP384R1,
            ];
            // We are the client and offer everything; rustls supports one.
            let facts = client_side(version, Kind::Ed25519, &all, vec![theirs]);
            assert_eq!(facts.ours.key_exchange_group(), ours, "{name}, client");
            assert_eq!(
                facts
                    .theirs
                    .negotiated_key_exchange_group()
                    .map(|g| g.name()),
                Some(name_of(ours)),
                "{name}, client: rustls disagrees"
            );
            // We are the server and support everything; rustls offers one.
            let facts = server_side(version, Kind::Ed25519, None, &all, vec![theirs]);
            assert_eq!(facts.ours.key_exchange_group(), ours, "{name}, server");
            assert_eq!(
                facts
                    .theirs
                    .negotiated_key_exchange_group()
                    .map(|g| g.name()),
                Some(name_of(ours)),
                "{name}, server: rustls disagrees"
            );
        }
    }
}

#[test]
fn a_hello_retry_request_is_reported_by_both_ends() {
    use rustls::crypto::ring::kx_group::{SECP256R1, X25519};

    // A client whose first share is X25519, to a server that only has P-256:
    // the server asks for a retry.
    let facts = client_side(
        &rustls::version::TLS13,
        Kind::P256,
        &[NamedGroup::X25519, NamedGroup::SecP256R1],
        vec![SECP256R1],
    );
    assert!(facts.ours.used_hello_retry_request(), "our client");
    assert_eq!(
        facts.theirs.handshake_kind(),
        Some(rustls::HandshakeKind::FullWithHelloRetryRequest),
        "rustls (the server) saw no retry"
    );
    assert_eq!(facts.ours.key_exchange_group(), NamedGroup::SecP256R1);

    // The same, with us as the server: rustls leads with X25519, we only have
    // P-256, and rustls reports that it had to retry.
    let facts = server_side(
        &rustls::version::TLS13,
        Kind::Ed25519,
        None,
        &[NamedGroup::SecP256R1],
        vec![X25519, SECP256R1],
    );
    assert!(facts.ours.used_hello_retry_request(), "our server");
    assert_eq!(
        facts.theirs.handshake_kind(),
        Some(rustls::HandshakeKind::FullWithHelloRetryRequest),
        "rustls (the client) saw no retry"
    );

    // No retry when the first share is usable, and never in TLS 1.2.
    let facts = client_side(
        &rustls::version::TLS13,
        Kind::P256,
        &[NamedGroup::SecP256R1],
        vec![SECP256R1],
    );
    assert!(!facts.ours.used_hello_retry_request());
    let facts = client_side(
        &rustls::version::TLS12,
        Kind::P256,
        &[NamedGroup::X25519, NamedGroup::SecP256R1],
        vec![SECP256R1],
    );
    assert!(!facts.ours.used_hello_retry_request());
}

#[test]
fn the_peer_scheme_is_the_one_its_key_signs_with() {
    use rustls::crypto::ring::kx_group::X25519;
    for (name, version) in VERSIONS {
        for kind in [Kind::P256, Kind::P384, Kind::Ed25519] {
            // We are the client: the scheme is the server's.
            let facts = client_side(
                version,
                kind,
                &[
                    NamedGroup::X25519,
                    NamedGroup::SecP256R1,
                    NamedGroup::SecP384R1,
                ],
                vec![X25519],
            );
            assert_eq!(
                facts.ours.peer_signature_scheme(),
                Some(kind.scheme()),
                "{name}, {kind:?}, as the client"
            );

            // We are the server and the client authenticated: it is the
            // client's. P-384 needs its curve among the 1.2 groups.
            let all = [
                NamedGroup::X25519,
                NamedGroup::SecP256R1,
                NamedGroup::SecP384R1,
            ];
            let facts = server_side(version, Kind::Ed25519, Some(kind), &all, vec![X25519]);
            assert_eq!(
                facts.ours.peer_signature_scheme(),
                Some(kind.scheme()),
                "{name}, {kind:?}, as the server"
            );
        }
        // No client certificate, nothing the client proved.
        let all = [NamedGroup::X25519, NamedGroup::SecP256R1];
        let facts = server_side(version, Kind::Ed25519, None, &all, vec![X25519]);
        assert_eq!(facts.ours.peer_signature_scheme(), None, "{name}");
    }
}
