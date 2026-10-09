//! SNI-based certificate selection in the native server — stage 12.
//!
//! The oracle is a client that checks the certificate against the name it
//! asked for: rustls verifies the chain and the name itself, so a handshake
//! that completes proves the server showed a certificate valid for that name,
//! and one that does not is a server that showed the wrong one. Both versions
//! run through the combined server with the peer pinned to one version.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::CertificateDer;
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{record_length, CipherSuite, ClientConfig};
use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, ServerConfigBoth, ServerHandshakeBoth,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::{ServerConfig, SniIdentity};
use rusty_tls::handrolled::server12::ServerConfig12;
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::x509::Certificate;

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

/// One CA and a leaf per name set, each with its own key.
struct World {
    root_der: Vec<u8>,
    default: Leaf,
    a: Leaf,
    wild: Leaf,
}

struct Leaf {
    chain: Vec<Vec<u8>>,
    key: SigningKey,
}

fn world() -> World {
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
        rcgen::DnValue::Utf8String("sni test root".to_string()),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");

    // `a` has a P-384 key and the others P-256, so a server that picked the
    // signature scheme from the default key rather than the chosen one would
    // be asked to sign with something the chosen key cannot.
    let leaf = |name: &str, p384: bool| {
        let algorithm = if p384 {
            &rcgen::PKCS_ECDSA_P384_SHA384
        } else {
            &rcgen::PKCS_ECDSA_P256_SHA256
        };
        let key = KeyPair::generate_for(algorithm).expect("leaf key");
        let mut params = CertificateParams::new(vec![name.to_string()]).expect("params");
        dated(&mut params);
        let cert = params.signed_by(&key, &root, &root_key).expect("leaf");
        Leaf {
            chain: vec![cert.der().to_vec(), root.der().to_vec()],
            key: if p384 {
                SigningKey::ecdsa_p384(&key.serialize_der())
            } else {
                SigningKey::ecdsa_p256(&key.serialize_der())
            }
            .expect("signing key"),
        }
    };
    World {
        root_der: root.der().to_vec(),
        default: leaf("default.example", false),
        a: leaf("a.example", true),
        wild: leaf("*.wild.example", false),
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

fn pump(client: &mut rustls::ClientConnection, input: &[u8]) -> (Vec<u8>, Option<String>) {
    let mut error = None;
    if !input.is_empty() {
        let mut cursor = std::io::Cursor::new(input);
        while client.read_tls(&mut cursor).unwrap_or(0) > 0 {
            if let Err(e) = client.process_new_packets() {
                error = Some(format!("{e:?}"));
                break;
            }
        }
    }
    if error.is_none() {
        if let Err(e) = client.process_new_packets() {
            error = Some(format!("{e:?}"));
        }
    }
    let mut out = Vec::new();
    while client.wants_write() {
        client.write_tls(&mut out).expect("write_tls");
    }
    (out, error)
}

/// What a client asking for `name` concluded, and what the server saw.
struct Outcome {
    /// `Some(error)` if rustls refused the server's certificate (or anything).
    client_error: Option<String>,
    /// The `host_name` our server read, if the handshake got that far.
    server_saw: Option<String>,
}

fn rustls_client_asks_for(version: Version, name: &str) -> Outcome {
    let world = world();
    let server_13 = |sni| ServerConfig {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: None,
        alpn: &[],
        sni,
    };
    let server_12 = |sni| ServerConfig12 {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        alpn: &[],
        sni,
    };
    let sni = [
        SniIdentity {
            names: &["a.example"],
            certificates: &world.a.chain,
            key: &world.a.key,
        },
        SniIdentity {
            names: &["*.wild.example"],
            certificates: &world.wild.chain,
            key: &world.wild.key,
        },
    ];
    let (tls13, tls12) = (server_13(&sni), server_12(&sni));
    let both = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(world.root_der.clone()))
        .expect("root");
    let config = rustls::ClientConfig::builder_with_protocol_versions(&[version])
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut client = rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from(name.to_string()).expect("name"),
    )
    .expect("client");

    let mut server = ServerHandshakeBoth::new(&both);
    let mut to_server = pump(&mut client, &[]).0;
    for _ in 0..16 {
        let mut reply = Vec::new();
        let mut stream = std::mem::take(&mut to_server);
        for record in take_records(&mut stream) {
            match server.read_record(&record) {
                Ok(bytes) => reply.extend_from_slice(&bytes),
                Err(error) => panic!("our server refused {name}: {error:?}"),
            }
        }
        let (more, error) = pump(&mut client, &reply);
        if error.is_some() {
            return Outcome {
                client_error: error,
                server_saw: None,
            };
        }
        to_server = more;
        if server.is_finished() && to_server.is_empty() {
            break;
        }
    }
    let established = server.into_connection().expect("finished");
    Outcome {
        client_error: None,
        server_saw: established.server_name().map(str::to_string),
    }
}

#[test]
fn a_name_selects_its_certificate_in_both_versions() {
    for (version_name, version) in VERSIONS {
        for name in ["a.example", "x.wild.example"] {
            let outcome = rustls_client_asks_for(version, name);
            assert_eq!(
                outcome.client_error, None,
                "{version_name}: {name} was shown the wrong certificate"
            );
            assert_eq!(outcome.server_saw.as_deref(), Some(name), "{version_name}");
        }
    }
}

#[test]
fn any_other_name_gets_the_default_certificate() {
    for (version_name, version) in VERSIONS {
        // The default's own name: a certificate valid for it was shown.
        let outcome = rustls_client_asks_for(version, "default.example");
        assert_eq!(outcome.client_error, None, "{version_name}");
        assert_eq!(
            outcome.server_saw.as_deref(),
            Some("default.example"),
            "{version_name}"
        );

        // A name nothing covers is shown the default, which is not valid for
        // it: the client's refusal is a *name* failure, and not a handshake
        // failure from a server that had nothing to say.
        let outcome = rustls_client_asks_for(version, "nobody.example");
        let error = outcome
            .client_error
            .unwrap_or_else(|| panic!("{version_name}: an uncovered name was served as valid"));
        assert!(
            error.contains("NotValidForName") || error.contains("NotValidForNameContext"),
            "{version_name}: expected a name mismatch on the default certificate, got {error}"
        );
    }
}

#[test]
fn a_wildcard_covers_exactly_one_label() {
    for (version_name, version) in VERSIONS {
        // The bare domain and a two-label subdomain are not covered, so the
        // default is shown and does not fit.
        for name in ["wild.example", "a.b.wild.example"] {
            let outcome = rustls_client_asks_for(version, name);
            let error = outcome.client_error.unwrap_or_else(|| {
                panic!("{version_name}: {name} was served a certificate valid for it")
            });
            assert!(
                error.contains("NotValidForName"),
                "{version_name}: {name}: {error}"
            );
        }
    }
}

/// Our own client sends the name as written. Matching ignores case on both
/// sides of the handshake, so a differently-cased name still finds its identity.
#[test]
fn matching_ignores_case() {
    let world = world();
    let sni = [SniIdentity {
        names: &["a.example"],
        certificates: &world.a.chain,
        key: &world.a.key,
    }];
    let tls13 = ServerConfig {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: None,
        alpn: &[],
        sni: &sni,
    };
    let tls12 = ServerConfig12 {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        alpn: &[],
        sni: &sni,
    };
    let both = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let root = Certificate::parse(&world.root_der).expect("root");
    let anchors = [TrustAnchor::from_certificate(&root)];
    let client_config = ClientConfigBoth::new(
        ClientConfig {
            server_name: ServerName::Dns("A.Example"),
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
    let (mut client, mut to_server) = ClientHandshakeBoth::start(&client_config).expect("start");
    let mut server = ServerHandshakeBoth::new(&both);
    for _ in 0..8 {
        let mut reply = Vec::new();
        let mut stream = std::mem::take(&mut to_server);
        for record in take_records(&mut stream) {
            reply.extend(server.read_record(&record).expect("server"));
        }
        let mut stream = reply;
        for record in take_records(&mut stream) {
            to_server.extend(client.read_record(&record).expect(
                "our client refused the certificate the server chose for a differently-cased name",
            ));
        }
        if client.is_finished() && server.is_finished() {
            break;
        }
    }
    let established = server.into_connection().expect("finished");
    assert_eq!(established.server_name(), Some("A.Example"));
}

#[test]
fn a_client_with_no_name_gets_the_default() {
    // An IP address is never sent as a server_name, so our client sends none.
    let world = world();
    let sni = [SniIdentity {
        names: &["a.example"],
        certificates: &world.a.chain,
        key: &world.a.key,
    }];
    let tls13 = ServerConfig {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: None,
        alpn: &[],
        sni: &sni,
    };
    let tls12 = ServerConfig12 {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        alpn: &[],
        sni: &sni,
    };
    let both = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let root = Certificate::parse(&world.root_der).expect("root");
    let anchors = [TrustAnchor::from_certificate(&root)];
    // The default certificate is for default.example, so an address cannot be
    // verified against it; the point here is only what the server presents and
    // reports, so the client's refusal is expected and its alert not needed.
    let client_config = ClientConfigBoth::new(
        ClientConfig {
            server_name: ServerName::Ip("192.0.2.1".parse().expect("ip")),
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
    let (mut client, hello) = ClientHandshakeBoth::start(&client_config).expect("start");
    let mut server = ServerHandshakeBoth::new(&both);
    let mut flight = server.read_record(&hello).expect("the server answers");
    let mut served_default = false;
    for record in take_records(&mut flight) {
        // The client reads the certificate and refuses the name: that it got
        // as far as a name check on the default chain is the observation.
        if let Err(error) = client.read_record(&record) {
            served_default =
                format!("{error:?}").contains("Path") || format!("{error:?}").contains("Name");
            break;
        }
    }
    assert!(
        served_default,
        "the server did not present the default certificate"
    );
}

/// Identities are tried in order and the first that covers the name wins, so a
/// broad entry ahead of a narrow one shadows it. The order is the caller's.
#[test]
fn the_first_matching_identity_wins() {
    let world = world();
    // `wild` is listed first and claims a.example too; its certificate is not
    // valid for that name, which is what shows it was the one chosen.
    let sni = [
        SniIdentity {
            names: &["a.example", "*.wild.example"],
            certificates: &world.wild.chain,
            key: &world.wild.key,
        },
        SniIdentity {
            names: &["a.example"],
            certificates: &world.a.chain,
            key: &world.a.key,
        },
    ];
    let tls13 = ServerConfig {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: None,
        alpn: &[],
        sni: &sni,
    };
    let root = Certificate::parse(&world.root_der).expect("root");
    let anchors = [TrustAnchor::from_certificate(&root)];
    let client_config = ClientConfig {
        server_name: ServerName::Dns("a.example"),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
        alpn: &[],
    };
    let (mut client, hello) =
        rusty_tls::handrolled::client::ClientHandshake::start(&client_config).expect("start");
    let mut server = rusty_tls::handrolled::server::ServerHandshake::new(&tls13);
    let mut flight = server.read_record(&hello).expect("the server answers");
    let refused = take_records(&mut flight)
        .iter()
        .any(|record| client.read_record(record).is_err());
    assert!(
        refused,
        "the later, exact identity was chosen over the earlier matching one"
    );
}

/// The selection is made again from the hello that follows a
/// HelloRetryRequest, which is a separate path in the server.
#[test]
fn the_identity_survives_a_hello_retry_request() {
    let world = world();
    let sni = [SniIdentity {
        names: &["a.example"],
        certificates: &world.a.chain,
        key: &world.a.key,
    }];
    // Only P-256: rustls sends an X25519 share first and is asked to retry.
    let tls13 = ServerConfig {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::SecP256R1],
        client_auth: None,
        tickets: None,
        alpn: &[],
        sni: &sni,
    };
    let tls12 = ServerConfig12 {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::SecP256R1],
        client_auth: None,
        alpn: &[],
        sni: &sni,
    };
    let both = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(world.root_der.clone()))
        .expect("root");
    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut client = rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from("a.example").expect("name"),
    )
    .expect("client");

    let mut server = ServerHandshakeBoth::new(&both);
    let mut to_server = pump(&mut client, &[]).0;
    let mut rounds = 0;
    for _ in 0..16 {
        let mut reply = Vec::new();
        let mut stream = std::mem::take(&mut to_server);
        for record in take_records(&mut stream) {
            reply.extend(server.read_record(&record).expect("server"));
        }
        if !reply.is_empty() {
            rounds += 1;
        }
        let (more, error) = pump(&mut client, &reply);
        assert_eq!(
            error, None,
            "the retried hello was shown the wrong identity"
        );
        to_server = more;
        if server.is_finished() && to_server.is_empty() {
            break;
        }
    }
    assert!(
        rounds >= 2,
        "no HelloRetryRequest happened, so nothing was tested"
    );
    assert_eq!(
        server.into_connection().expect("finished").server_name(),
        Some("a.example")
    );
}

/// Only a `host_name` entry is a name; other types are skipped, and a name that
/// is not UTF-8 is malformed.
#[test]
fn only_a_host_name_entry_is_a_name() {
    use rusty_tls::handrolled::handshake::{extension, host_name_from, Extension};

    let list = |entries: &[(u8, &[u8])]| {
        let mut body = Vec::new();
        for (kind, name) in entries {
            body.push(*kind);
            body.extend_from_slice(&(name.len() as u16).to_be_bytes());
            body.extend_from_slice(name);
        }
        let mut out = (body.len() as u16).to_be_bytes().to_vec();
        out.extend(body);
        out
    };
    let name_of = |body: &[u8]| {
        host_name_from(&[Extension {
            typ: extension::SERVER_NAME,
            data: body,
        }])
        .map(|n| n.map(str::to_string))
    };
    assert_eq!(
        name_of(&list(&[(0, b"a.example")])),
        Ok(Some("a.example".into()))
    );
    // A name of another type first, then the host name.
    assert_eq!(
        name_of(&list(&[(1, b"other"), (0, b"a.example")])),
        Ok(Some("a.example".into()))
    );
    assert_eq!(name_of(&list(&[(1, b"other")])), Ok(None));
    assert!(name_of(&list(&[(0, &[0xff, 0xfe])])).is_err());
    assert_eq!(host_name_from(&[]), Ok(None));
}

/// RFC 6066 section 3: a server that used the name to choose answers with an
/// empty `server_name`; one that fell back to the default did not use it and
/// says nothing. Read through our own client, in both versions: the combined
/// client lands on TLS 1.3, the standalone 1.2 client forces TLS 1.2.
#[test]
fn the_name_is_acknowledged_only_when_it_chose_an_identity() {
    use rusty_tls::handrolled::client12::{ClientConfig12, ClientHandshake12};

    let world = world();
    let sni = [SniIdentity {
        names: &["a.example"],
        certificates: &world.a.chain,
        key: &world.a.key,
    }];
    let tls13 = ServerConfig {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: None,
        alpn: &[],
        sni: &sni,
    };
    let tls12 = ServerConfig12 {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        alpn: &[],
        sni: &sni,
    };
    let both = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let root = Certificate::parse(&world.root_der).expect("root");
    let anchors = [TrustAnchor::from_certificate(&root)];

    for (name, expected) in [("a.example", true), ("default.example", false)] {
        // TLS 1.3, through the combined client.
        let config = ClientConfigBoth::new(
            ClientConfig {
                server_name: ServerName::Dns(name),
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
        let mut server = ServerHandshakeBoth::new(&both);
        for _ in 0..8 {
            let mut from_server = Vec::new();
            let mut stream = std::mem::take(&mut to_server);
            for record in take_records(&mut stream) {
                from_server.extend(server.read_record(&record).expect("server"));
            }
            for record in take_records(&mut from_server) {
                to_server.extend(client.read_record(&record).expect("client"));
            }
            if client.is_finished() {
                break;
            }
        }
        let established = client.into_connection().expect("finished");
        assert_eq!(
            established.server_name_acknowledged(),
            expected,
            "TLS 1.3, {name}"
        );

        // TLS 1.2, through the standalone client.
        let config12 = ClientConfig12 {
            server_name: ServerName::Dns(name),
            anchors: &anchors,
            path: options(),
            // The keys are P-256 and P-384, and a TLS 1.2 ECDSA signature needs
            // its curve among the client's groups.
            groups: &[
                NamedGroup::X25519,
                NamedGroup::SecP256R1,
                NamedGroup::SecP384R1,
            ],
            cipher_suites: CipherSuite12::SUPPORTED,
            identity: None,
            alpn: &[],
        };
        let (mut client, mut to_server) = ClientHandshake12::start(&config12).expect("start");
        let mut server = ServerHandshakeBoth::new(&both);
        for _ in 0..8 {
            let mut from_server = Vec::new();
            let mut stream = std::mem::take(&mut to_server);
            for record in take_records(&mut stream) {
                from_server.extend(server.read_record(&record).expect("server"));
            }
            for record in take_records(&mut from_server) {
                to_server.extend(client.read_record(&record).expect("client"));
            }
            if client.is_finished() {
                break;
            }
        }
        let connection = client.into_connection().expect("finished");
        assert_eq!(
            connection.server_name_acknowledged(),
            expected,
            "TLS 1.2, {name}"
        );
    }
}

/// A ticket is bound to the chain it was issued under. One issued while serving
/// `a.example` resumes for `a.example`, and is not honoured when the same client
/// presents it to the default identity: a ticket is not a way to be shown a
/// certificate it never saw.
#[test]
fn a_ticket_is_bound_to_the_identity_that_issued_it() {
    use rusty_tls::handrolled::client::{
        ClientHandshake, Connection, Incoming, Resumption, Session,
    };
    use rusty_tls::handrolled::server::{ServerHandshake, Tickets};
    use rusty_tls::handrolled::ticket::{TicketKey, TicketKeys};

    let world = world();
    let ticket_key = TicketKey::generate().expect("a ticket key");
    let tickets = Tickets {
        keys: TicketKeys {
            current: &ticket_key,
            previous: &[],
        },
        now: options().time,
        lifetime: 7200,
        max_age_skew_ms: None,
        count: 1,
    };
    let sni = [SniIdentity {
        names: &["a.example"],
        certificates: &world.a.chain,
        key: &world.a.key,
    }];
    let server_config = ServerConfig {
        certificates: &world.default.chain,
        key: &world.default.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
        client_auth: None,
        tickets: Some(&tickets),
        alpn: &[],
        sni: &sni,
    };
    let root = Certificate::parse(&world.root_der).expect("root");
    let anchors = [TrustAnchor::from_certificate(&root)];

    // One TLS 1.3 handshake for `name`, offering `session` if there is one.
    // Returns the connection and the tickets that followed.
    let run = |name: &str, session: Option<&Session>| -> (Connection, Vec<Session>) {
        let config = ClientConfig {
            server_name: ServerName::Dns(name),
            anchors: &anchors,
            path: options(),
            groups: &[NamedGroup::X25519],
            cipher_suites: CipherSuite::SUPPORTED,
            identity: None,
            resumption: session.map(|session| Resumption {
                session,
                age_ms: 1_000,
            }),
            alpn: &[],
        };
        let (mut client, mut to_server) = ClientHandshake::start(&config).expect("start");
        let mut server = ServerHandshake::new(&server_config);
        let mut last = Vec::new();
        for _ in 0..8 {
            let mut from_server = Vec::new();
            let mut stream = std::mem::take(&mut to_server);
            for record in take_records(&mut stream) {
                from_server = server.read_record(&record).expect("server");
            }
            last = from_server.clone();
            for record in take_records(&mut from_server) {
                to_server.extend(client.read_record(&record).expect("client"));
            }
            if client.is_finished() {
                // Deliver the client's Finished so the server issues tickets.
                let mut stream = std::mem::take(&mut to_server);
                for record in take_records(&mut stream) {
                    last = server.read_record(&record).expect("server");
                }
                break;
            }
        }
        let mut connection = client.into_connection().expect("finished");
        let mut sessions = Vec::new();
        for record in take_records(&mut last) {
            if let Incoming::Tickets(more) = connection.read(&record).expect("a ticket") {
                sessions.extend(more);
            }
        }
        (connection, sessions)
    };

    let (first, sessions) = run("a.example", None);
    assert!(!first.resumed());
    let session = sessions.first().expect("a ticket was issued");

    let (again, _) = run("a.example", Some(session));
    assert!(
        again.resumed(),
        "a ticket did not resume for its own identity"
    );

    let (other, _) = run("default.example", Some(session));
    assert!(
        !other.resumed(),
        "a ticket issued under a.example resumed against the default identity"
    );
}
