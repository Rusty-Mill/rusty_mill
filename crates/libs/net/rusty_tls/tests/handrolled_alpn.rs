//! ALPN (RFC 7301) in the native engine — stage 11.
//!
//! The oracle is a real peer. A completed handshake with rustls in each
//! direction and each version shows the extension is encoded and answered the
//! way another implementation reads it; the refusals show the engine says
//! `no_application_protocol` when nothing is shared, and that a client does
//! not believe a server that selects what it never offered.
//!
//! Both versions run through the combined machines, with the peer pinned to
//! one version, because that is how a caller meets them and it exercises the
//! TLS 1.3 EncryptedExtensions path and the TLS 1.2 ServerHello path with one
//! driver.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{
    record_length, AlertDescription, CipherSuite, ClientConfig, ClientError,
};
use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::handshake::{
    choose_alpn, encode_alpn_offer, extension, AlpnChoice, Extension,
};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, Established, ServerConfigBoth, ServerHandshakeBoth,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::{ServerConfig, ServerError};
use rusty_tls::handrolled::server12::ServerConfig12;
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::x509::Certificate;

const SERVER: &str = "alpn.example";

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
        rcgen::DnValue::Utf8String("alpn test root".to_string()),
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

/// Feed bytes to a rustls connection and collect what it wants to send, with
/// the error it raised, if any: a refusal still leaves an alert queued.
fn pump(connection: &mut rustls::Connection, input: &[u8]) -> (Vec<u8>, Option<rustls::Error>) {
    let mut error = None;
    if !input.is_empty() {
        let mut cursor = std::io::Cursor::new(input);
        while connection.read_tls(&mut cursor).unwrap_or(0) > 0 {
            if let Err(e) = connection.process_new_packets() {
                error = Some(e);
                break;
            }
        }
    }
    if error.is_none() {
        if let Err(e) = connection.process_new_packets() {
            error = Some(e);
        }
    }
    let mut out = Vec::new();
    while connection.wants_write() {
        connection.write_tls(&mut out).expect("write_tls");
    }
    (out, error)
}

// ---------------------------------------------------------------------------
// Our client, a rustls server
// ---------------------------------------------------------------------------

/// What our client concluded, and what alert (if any) rustls queued.
struct ClientRun {
    result: Result<Option<Vec<u8>>, ClientError>,
}

fn client_against_rustls(
    version: Version,
    server_alpn: &[&[u8]],
    client_alpn: &[&[u8]],
) -> ClientRun {
    let pki = pki();
    let mut server_config = rustls::ServerConfig::builder_with_protocol_versions(&[version])
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
    server_config.alpn_protocols = server_alpn.iter().map(|p| p.to_vec()).collect();
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
            alpn: client_alpn,
        },
        CipherSuite12::SUPPORTED,
    );

    let (mut client, mut to_server) = match ClientHandshakeBoth::start(&config) {
        Ok(started) => started,
        Err(error) => return ClientRun { result: Err(error) },
    };
    for _ in 0..16 {
        let (from_server, _) = pump(&mut server, &to_server);
        to_server.clear();
        let mut stream = from_server;
        for record in take_records(&mut stream) {
            match client.read_record(&record) {
                Ok(reply) => to_server.extend_from_slice(&reply),
                Err(error) => return ClientRun { result: Err(error) },
            }
        }
        if client.is_finished() {
            let established = client.into_connection().expect("finished");
            return ClientRun {
                result: Ok(established.alpn_protocol().map(<[u8]>::to_vec)),
            };
        }
    }
    ClientRun {
        result: Err(ClientError::Failed),
    }
}

#[test]
fn our_client_gets_the_protocol_a_rustls_server_picks() {
    for (name, version) in VERSIONS {
        // rustls selects by *its* preference: h2 over http/1.1.
        let run = client_against_rustls(version, &[b"h2", b"http/1.1"], &[b"http/1.1", b"h2"]);
        assert_eq!(
            run.result.unwrap_or_else(|e| panic!("{name}: {e}")),
            Some(b"h2".to_vec()),
            "{name}"
        );
        // One in common.
        let run = client_against_rustls(version, &[b"h2", b"http/1.1"], &[b"http/1.1"]);
        assert_eq!(
            run.result.unwrap_or_else(|e| panic!("{name}: {e}")),
            Some(b"http/1.1".to_vec()),
            "{name}"
        );
    }
}

#[test]
fn a_client_that_offers_nothing_ends_up_with_nothing() {
    for (name, version) in VERSIONS {
        let run = client_against_rustls(version, &[b"h2"], &[]);
        assert_eq!(
            run.result.unwrap_or_else(|e| panic!("{name}: {e}")),
            None,
            "{name}: ALPN appeared from nowhere"
        );
    }
}

#[test]
fn a_rustls_server_with_nothing_in_common_refuses_and_we_read_the_alert() {
    for (name, version) in VERSIONS {
        let run = client_against_rustls(version, &[b"h2"], &[b"h3"]);
        match run.result {
            Err(ClientError::PeerAlert(alert)) => assert_eq!(
                alert.description,
                AlertDescription::NO_APPLICATION_PROTOCOL,
                "{name}"
            ),
            other => panic!("{name}: expected the no_application_protocol alert, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Our server, a rustls client
// ---------------------------------------------------------------------------

struct ServerRun {
    /// Our side: the protocol selected, or the error.
    ours: Result<Option<Vec<u8>>, ServerError>,
    /// rustls's view of what was selected.
    theirs: Option<Vec<u8>>,
}

fn server_against_rustls(
    version: Version,
    client_alpn: &[&[u8]],
    server_alpn: &[&[u8]],
) -> ServerRun {
    let pki = pki();
    let key = SigningKey::ecdsa_p256(&pki.pkcs8).expect("key");
    let server_13 = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: None,
        alpn: server_alpn,
    };
    let server_12 = ServerConfig12 {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        alpn: server_alpn,
    };
    let both = ServerConfigBoth {
        tls13: &server_13,
        tls12: &server_12,
    };

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(pki.root_der.clone()))
        .expect("root");
    let mut client_config = rustls::ClientConfig::builder_with_protocol_versions(&[version])
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = client_alpn.iter().map(|p| p.to_vec()).collect();
    let mut client: rustls::Connection = rustls::ClientConnection::new(
        Arc::new(client_config),
        rustls::pki_types::ServerName::try_from(SERVER).expect("name"),
    )
    .expect("client")
    .into();

    let mut server = ServerHandshakeBoth::new(&both);
    let mut to_server = pump(&mut client, &[]).0;
    for _ in 0..16 {
        let mut reply = Vec::new();
        let mut stream = std::mem::take(&mut to_server);
        for record in take_records(&mut stream) {
            match server.read_record(&record) {
                Ok(bytes) => reply.extend_from_slice(&bytes),
                Err(error) => {
                    // Let rustls read the alert, if one is framed.
                    if let Some(alert) = server.alert_record(&error) {
                        let _ = pump(&mut client, &alert);
                    }
                    return ServerRun {
                        ours: Err(error),
                        theirs: client.alpn_protocol().map(<[u8]>::to_vec),
                    };
                }
            }
        }
        let (more, error) = pump(&mut client, &reply);
        assert!(error.is_none(), "rustls refused our server: {error:?}");
        to_server = more;
        if server.is_finished() && to_server.is_empty() {
            break;
        }
    }
    let established: Established = server.into_connection().expect("finished");
    ServerRun {
        ours: Ok(established.alpn_protocol().map(<[u8]>::to_vec)),
        theirs: client.alpn_protocol().map(<[u8]>::to_vec),
    }
}

#[test]
fn our_server_selects_by_its_own_preference_and_rustls_agrees() {
    for (name, version) in VERSIONS {
        // The client prefers h2; this server prefers http/1.1. RFC 7301 leaves
        // the choice to the server, and the answer says whose list wins.
        let run = server_against_rustls(version, &[b"h2", b"http/1.1"], &[b"http/1.1", b"h2"]);
        assert_eq!(
            run.ours.unwrap_or_else(|e| panic!("{name}: {e}")),
            Some(b"http/1.1".to_vec()),
            "{name}"
        );
        assert_eq!(
            run.theirs,
            Some(b"http/1.1".to_vec()),
            "{name}: rustls disagrees"
        );
    }
}

#[test]
fn our_server_without_alpn_ignores_a_clients_offer() {
    for (name, version) in VERSIONS {
        let run = server_against_rustls(version, &[b"h2"], &[]);
        assert_eq!(
            run.ours.unwrap_or_else(|e| panic!("{name}: {e}")),
            None,
            "{name}"
        );
        assert_eq!(run.theirs, None, "{name}");
    }
}

#[test]
fn our_server_refuses_a_client_it_shares_nothing_with() {
    for (name, version) in VERSIONS {
        let run = server_against_rustls(version, &[b"h3"], &[b"h2", b"http/1.1"]);
        let error = run.ours.expect_err(&format!(
            "{name}: a handshake with no shared protocol completed"
        ));
        assert_eq!(error, ServerError::NoApplicationProtocol, "{name}");
        assert_eq!(
            error.alert(),
            Some(AlertDescription::NO_APPLICATION_PROTOCOL),
            "{name}"
        );
    }
}

#[test]
fn a_client_that_offers_no_alpn_to_a_server_that_has_some_completes_without() {
    for (name, version) in VERSIONS {
        let run = server_against_rustls(version, &[], &[b"h2"]);
        assert_eq!(
            run.ours.unwrap_or_else(|e| panic!("{name}: {e}")),
            None,
            "{name}"
        );
    }
}

// ---------------------------------------------------------------------------
// The policy and the wire format, without a peer
// ---------------------------------------------------------------------------

fn offer(protocols: &[&[u8]]) -> Vec<u8> {
    encode_alpn_offer(protocols)
}

#[test]
fn the_servers_list_decides_and_an_absent_offer_is_not_a_mismatch() {
    let ours: [&[u8]; 2] = [b"h2", b"http/1.1"];
    let body = offer(&[b"http/1.1", b"h2"]);
    let with = [Extension {
        typ: extension::ALPN,
        data: &body,
    }];
    assert_eq!(choose_alpn(&ours, &with), Ok(AlpnChoice::Selected(b"h2")));
    assert_eq!(choose_alpn(&ours, &[]), Ok(AlpnChoice::Unused));
    assert_eq!(choose_alpn(&[], &with), Ok(AlpnChoice::Unused));

    let other = offer(&[b"h3"]);
    let nothing = [Extension {
        typ: extension::ALPN,
        data: &other,
    }];
    assert_eq!(choose_alpn(&ours, &nothing), Ok(AlpnChoice::NoOverlap));
}

#[test]
fn a_malformed_offer_is_malformed_even_to_a_server_that_would_ignore_it() {
    // An empty list, an empty name, trailing bytes, a short name.
    let bad: [&[u8]; 4] = [
        &[0x00, 0x00],
        &[0x00, 0x01, 0x00],
        &[0x00, 0x03, 0x02, b'h', b'2', 0xff],
        &[0x00, 0x04, 0x05, b'h', b'2', b'x'],
    ];
    for body in bad {
        let extensions = [Extension {
            typ: extension::ALPN,
            data: body,
        }];
        assert!(
            choose_alpn(&[], &extensions).is_err(),
            "{body:?} was accepted"
        );
        assert!(choose_alpn(&[b"h2"], &extensions).is_err(), "{body:?}");
    }
}

/// A server's answer names exactly one protocol (RFC 7301 section 3.1): a list
/// of two is a malformed answer, not a choice to make on the server's behalf.
#[test]
fn an_answer_naming_two_protocols_is_malformed() {
    use rusty_tls::handrolled::handshake::{encode_alpn_selection, parse_alpn_selection};

    assert_eq!(
        parse_alpn_selection(&encode_alpn_selection(b"h2")),
        Ok(&b"h2"[..])
    );
    let two = offer(&[b"h2", b"http/1.1"]);
    assert!(parse_alpn_selection(&two).is_err());
    assert!(parse_alpn_selection(&[0x00, 0x00]).is_err());
}
