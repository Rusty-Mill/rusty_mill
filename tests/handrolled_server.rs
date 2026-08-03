//! The TLS 1.3 server handshake — stage 5.
//!
//! # The oracle, and why it is the one that counts
//!
//! [`a_real_rustls_client_completes_a_handshake_with_this_server`] is the test
//! this file rests on, for the mirror of the reason 3c-ii's interop test was:
//! a handshake is a mutual computation, so a suite where both sides are this
//! code cannot catch a wrong secret or a mis-ordered transcript. `rustls` has
//! not read this implementation.
//!
//! It is a *better* oracle here than it was for the client, in one specific
//! way. A client's mistakes mostly make it refuse things it should accept — a
//! failure that is loud. A server's mistakes can make it *produce* something
//! wrong: a signature over the wrong bytes, a Finished over the wrong
//! transcript, a certificate chain in the wrong order. Those are only visible
//! to somebody checking, and `rustls` checks.
//!
//! # The other half
//!
//! Interop proves the server can complete a handshake with a correct client.
//! It says nothing about what the server does with an incorrect one, and a
//! server answers whoever connects. The refusal tests drive hostile and
//! malformed ClientHellos and require each to be turned away — with the right
//! alert, since a server that fails silently leaves every client guessing.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, ServerName as RustlsName};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{
    record_length, AlertDescription, CipherSuite, ClientConfig, ClientHandshake, Incoming,
};
use rusty_tls::handrolled::handshake::{messages, ClientHello, HandshakeType, Message};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::{ServerConfig, ServerError, ServerHandshake};
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::x509::Certificate;

const SERVER: &str = "handrolled.example";
const NOT_BEFORE: i64 = 1_577_836_800; // 2020-01-01
const NOT_AFTER: i64 = 1_893_456_000; // 2030-01-01

fn options() -> PathOptions {
    PathOptions {
        time: 1_800_000_000,
        max_path_length: 8,
        max_signature_checks: 64,
        required_eku: None,
    }
}

// ---------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------

struct Pki {
    root_der: Vec<u8>,
    chain: Vec<Vec<u8>>,
    leaf_pkcs8: Vec<u8>,
}

fn pki(algorithm: &'static rcgen::SignatureAlgorithm) -> Pki {
    let dated = |params: &mut CertificateParams| {
        params.not_before = OffsetDateTime::from_unix_timestamp(NOT_BEFORE).expect("not_before");
        params.not_after = OffsetDateTime::from_unix_timestamp(NOT_AFTER).expect("not_after");
    };

    let root_key = KeyPair::generate_for(algorithm).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    root_params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("handrolled server test root".to_string()),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");

    let leaf_key = KeyPair::generate_for(algorithm).expect("leaf key");
    let mut leaf_params = CertificateParams::new(vec![SERVER.to_string()]).expect("params");
    dated(&mut leaf_params);
    let leaf = leaf_params
        .signed_by(&leaf_key, &root, &root_key)
        .expect("leaf");

    Pki {
        root_der: root.der().to_vec(),
        chain: vec![leaf.der().to_vec(), root.der().to_vec()],
        leaf_pkcs8: leaf_key.serialize_der(),
    }
}

fn signing_key(pki: &Pki, algorithm: &'static rcgen::SignatureAlgorithm) -> SigningKey {
    if algorithm == &rcgen::PKCS_ECDSA_P384_SHA384 {
        SigningKey::ecdsa_p384(&pki.leaf_pkcs8).expect("P-384 key")
    } else if algorithm == &rcgen::PKCS_ED25519 {
        SigningKey::ed25519(&pki.leaf_pkcs8).expect("Ed25519 key")
    } else {
        SigningKey::ecdsa_p256(&pki.leaf_pkcs8).expect("P-256 key")
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

// ---------------------------------------------------------------------------
// Interop — the test this file rests on
// ---------------------------------------------------------------------------

fn rustls_client(pki: &Pki) -> rustls::ClientConnection {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(pki.root_der.clone()))
        .expect("the root is acceptable to rustls");

    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots)
        .with_no_client_auth();

    rustls::ClientConnection::new(
        Arc::new(config),
        RustlsName::try_from(SERVER).expect("name"),
    )
    .expect("client connection")
}

/// Drive a `rustls` client against this server to completion.
fn interop(algorithm: &'static rcgen::SignatureAlgorithm) -> rustls::ClientConnection {
    let pki = pki(algorithm);
    let key = signing_key(&pki, algorithm);
    let config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[
            NamedGroup::X25519,
            NamedGroup::SecP256R1,
            NamedGroup::SecP384R1,
        ],
    };
    let mut server = ServerHandshake::new(&config);
    let mut client = rustls_client(&pki);

    for _ in 0..8 {
        // Whatever the client wants to send.
        let mut to_server = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut to_server).expect("write_tls");
        }

        let mut from_server = Vec::new();
        for record in take_records(&mut to_server) {
            from_server.extend_from_slice(
                &server
                    .read_record(&record)
                    .unwrap_or_else(|e| panic!("server refused a rustls client: {e}")),
            );
        }

        if !from_server.is_empty() {
            let mut cursor = std::io::Cursor::new(&from_server);
            while client.read_tls(&mut cursor).expect("read_tls") > 0 {
                client.process_new_packets().expect("process_new_packets");
            }
            client.process_new_packets().expect("process_new_packets");
        }

        if server.is_finished() && !client.is_handshaking() {
            return client;
        }
        if from_server.is_empty() && !client.wants_write() {
            break;
        }
    }
    panic!("the handshake did not complete");
}

/// A real `rustls` client completes a handshake with this server, and both
/// sides agree well enough to carry data.
///
/// See the module docs on why this is worth more than any number of tests
/// where both sides are this code — and why it matters more for a server than
/// it did for the client.
#[test]
fn a_real_rustls_client_completes_a_handshake_with_this_server() {
    let client = interop(&rcgen::PKCS_ECDSA_P256_SHA256);
    assert!(!client.is_handshaking(), "rustls is still handshaking");

    let negotiated = client.protocol_version().expect("a version was negotiated");
    assert_eq!(negotiated, rustls::ProtocolVersion::TLSv1_3);

    // rustls validated the chain against the root it was given, which means
    // the certificate message and the CertificateVerify signature both checked
    // out on a peer that did not write them.
    assert!(
        client.peer_certificates().is_some_and(|c| c.len() == 2),
        "rustls did not receive the chain"
    );
}

/// Every key type this server can sign with, checked by a peer that verifies.
///
/// A server's mistakes can be *productive* — a signature over the wrong bytes
/// looks fine until somebody checks it — so each signing path needs a checker.
#[test]
fn every_signing_key_produces_a_signature_rustls_accepts() {
    for algorithm in [
        &rcgen::PKCS_ECDSA_P256_SHA256,
        &rcgen::PKCS_ECDSA_P384_SHA384,
        &rcgen::PKCS_ED25519,
    ] {
        let client = interop(algorithm);
        assert!(
            !client.is_handshaking(),
            "{algorithm:?} did not complete against rustls"
        );
    }
}

/// This crate's own client against this crate's own server.
///
/// Circular, and included anyway for one narrow thing it can show that the
/// interop test cannot: that both halves agree end to end including the
/// post-handshake path. It is a supplement to the `rustls` test, never a
/// substitute — if this passed and the interop test failed, this would be
/// the one that was wrong.
#[test]
fn this_clients_handshake_with_this_server_carries_data_both_ways() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = signing_key(&pki, &rcgen::PKCS_ECDSA_P256_SHA256);
    let server_config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
    };
    let mut server = ServerHandshake::new(&server_config);

    let root = Certificate::parse(&pki.root_der).expect("root parses");
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let client_config = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
    };

    let (mut client, hello) = ClientHandshake::start(&client_config).expect("start");
    let mut to_server = hello;

    for _ in 0..8 {
        let mut to_client = Vec::new();
        for record in take_records(&mut to_server) {
            to_client.extend_from_slice(&server.read_record(&record).expect("server"));
        }
        to_server.clear();
        for record in take_records(&mut to_client) {
            to_server.extend_from_slice(&client.read_record(&record).expect("client"));
        }
        if client.is_finished() && !to_server.is_empty() {
            for record in take_records(&mut to_server) {
                server.read_record(&record).expect("server finished");
            }
            break;
        }
    }

    assert!(server.is_finished(), "the server did not finish");
    assert!(client.is_finished(), "the client did not finish");

    let mut client_connection = client.into_connection().expect("client connection");
    let mut server_connection = server.into_connection().expect("server connection");

    let record = client_connection
        .write(b"hello from the client")
        .expect("write");
    assert_eq!(
        server_connection.read(&record).expect("server reads"),
        Incoming::Application(b"hello from the client".to_vec())
    );

    let record = server_connection
        .write(b"hello from the server")
        .expect("write");
    assert_eq!(
        client_connection.read(&record).expect("client reads"),
        Incoming::Application(b"hello from the server".to_vec())
    );
}

// ---------------------------------------------------------------------------
// Refusals — a server answers whoever connects
// ---------------------------------------------------------------------------

/// How to bend a genuine ClientHello out of shape.
///
/// Owned rather than a closure over borrowed buffers: a `ClientHello` borrows
/// the record it was parsed from, which lives inside [`client_hello`], so a
/// closure that supplied replacement extension data would have to promise it
/// for `'static`.
#[derive(Default)]
struct Edit {
    /// Extensions to drop entirely.
    remove: Vec<u16>,
    /// Extensions to replace the body of.
    replace: Vec<(u16, Vec<u8>)>,
    /// Cipher suites to offer instead.
    cipher_suites: Option<Vec<u16>>,
}

/// Build a genuine ClientHello with this crate's client, then apply `edit`.
///
/// Starting from a real one matters: a hand-built ClientHello would be testing
/// the server against whatever this test file believes a ClientHello looks
/// like, and the interesting refusals are all one field away from correct.
fn client_hello(edit: Edit) -> Vec<u8> {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let root = Certificate::parse(&pki.root_der).expect("parses");
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let config = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
    };
    let (_, record) = ClientHandshake::start(&config).expect("start");
    let parsed = messages(&record[5..]).expect("parses");
    let mut hello = ClientHello::parse(parsed[0].body).expect("parses");

    hello.extensions.retain(|e| !edit.remove.contains(&e.typ));
    for (typ, data) in &edit.replace {
        for extension in &mut hello.extensions {
            if extension.typ == *typ {
                extension.data = data;
            }
        }
    }
    if let Some(suites) = edit.cipher_suites {
        hello.cipher_suites = suites;
    }

    let encoded = Message::encode(HandshakeType::ClientHello, &hello.encode());
    let mut out = vec![22u8, 0x03, 0x01];
    out.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
    out.extend_from_slice(&encoded);
    out
}

/// Run one ClientHello at a fresh server and report what happened.
fn refuse(record: &[u8]) -> ServerError {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = signing_key(&pki, &rcgen::PKCS_ECDSA_P256_SHA256);
    let config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: &[CipherSuite::TLS_AES_128_GCM_SHA256],
        groups: &[NamedGroup::X25519],
    };
    let mut server = ServerHandshake::new(&config);
    server
        .read_record(record)
        .expect_err("the server accepted a ClientHello it should not have")
}

/// A client that does not offer TLS 1.3 is turned away with
/// `protocol_version` — the alert stage 4a taught the client half to read.
#[test]
fn a_client_that_does_not_offer_tls13_gets_a_protocol_version_alert() {
    use rusty_tls::handrolled::handshake::extension;

    let record = client_hello(Edit {
        remove: vec![extension::SUPPORTED_VERSIONS],
        ..Edit::default()
    });

    let error = refuse(&record);
    assert_eq!(error, ServerError::NotTls13);
    assert_eq!(error.alert(), Some(AlertDescription::PROTOCOL_VERSION));
}

/// `supported_versions` that lists only older versions is the same refusal.
///
/// A separate case from the extension being absent, because a client offering
/// TLS 1.2 explicitly is a different thing from one that predates the
/// extension — and a parser that stopped at "the extension is present" would
/// pass the test above and accept this.
#[test]
fn a_client_offering_only_older_versions_is_refused() {
    use rusty_tls::handrolled::handshake::extension;
    use rusty_tls::handrolled::wire::Writer;

    let mut versions = Writer::new();
    versions.vector_u8(|w| {
        w.u16(0x0303);
        w.u16(0x0302);
    });
    let record = client_hello(Edit {
        replace: vec![(extension::SUPPORTED_VERSIONS, versions.into_vec())],
        ..Edit::default()
    });

    let error = refuse(&record);
    assert_eq!(error, ServerError::NotTls13);
}

/// No cipher suite in common is a `handshake_failure`, not a silent pick.
#[test]
fn a_client_with_no_shared_cipher_suite_is_refused() {
    let record = client_hello(Edit {
        // The server offers only 0x1301.
        cipher_suites: Some(vec![0x1302, 0x1303]),
        ..Edit::default()
    });

    let error = refuse(&record);
    assert_eq!(error, ServerError::NoSharedCipherSuite);
    assert_eq!(error.alert(), Some(AlertDescription::HANDSHAKE_FAILURE));
}

/// No signature scheme in common is refused rather than signed anyway.
///
/// A server that signed with a scheme the client did not offer would produce
/// a signature the client is obliged to reject, and the failure would surface
/// three messages later looking like a transcript bug.
#[test]
fn a_client_with_no_shared_signature_scheme_is_refused() {
    use rusty_tls::handrolled::handshake::extension;
    use rusty_tls::handrolled::wire::Writer;

    // Only ed25519 and RSA-PSS; the server's key is P-256.
    let mut schemes = Writer::new();
    schemes.vector_u16(|w| {
        w.u16(0x0807);
        w.u16(0x0804);
    });
    let record = client_hello(Edit {
        replace: vec![(extension::SIGNATURE_ALGORITHMS, schemes.into_vec())],
        ..Edit::default()
    });

    let error = refuse(&record);
    assert_eq!(error, ServerError::NoSharedSignatureScheme);
}

/// A client whose `key_share` names no group the server has is refused.
///
/// A HelloRetryRequest would be the better answer; it is not implemented, and
/// this pins the behaviour that exists rather than the one that should.
#[test]
fn a_client_with_no_shared_group_is_refused() {
    use rusty_tls::handrolled::handshake::extension;

    let record = client_hello(Edit {
        remove: vec![extension::KEY_SHARE],
        ..Edit::default()
    });

    let error = refuse(&record);
    assert_eq!(error, ServerError::NoSharedGroup);
    assert_eq!(error.alert(), Some(AlertDescription::HANDSHAKE_FAILURE));
}

/// Anything other than a ClientHello first is refused.
#[test]
fn a_server_will_not_start_from_any_other_message() {
    let body = Message::encode(HandshakeType::Finished, &[0u8; 32]);
    let mut record = vec![22u8, 0x03, 0x01];
    record.extend_from_slice(&(body.len() as u16).to_be_bytes());
    record.extend_from_slice(&body);

    assert!(matches!(
        refuse(&record),
        ServerError::UnexpectedMessage {
            expected: "ClientHello",
            got: HandshakeType::Finished,
        }
    ));
}

/// A malformed ClientHello is a `decode_error`, and does not panic.
#[test]
fn a_malformed_client_hello_is_refused() {
    for body in [vec![], vec![0x03, 0x03], vec![0xff; 40]] {
        let record = if body.first() == Some(&22) {
            body
        } else {
            let encoded = Message::encode(HandshakeType::ClientHello, &body);
            let mut out = vec![22u8, 0x03, 0x01];
            out.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
            out.extend_from_slice(&encoded);
            out
        };
        let error = refuse(&record);
        assert!(
            error.alert().is_some(),
            "a malformed ClientHello produced no alert: {error:?}"
        );
    }
}

/// A ClientHello cut short is *waited for*, not refused.
///
/// The first draft of the test above lumped this in with malformed input and
/// failed, correctly: a truncated message is not malformed, it is incomplete,
/// and handshake messages span records routinely. Refusing one would break
/// every peer whose ClientHello did not fit in a single record.
///
/// The distinction is the whole reason `handshake::complete_prefix` exists,
/// and it deserves a test that says so rather than being folded into a
/// rejection suite where a bug would look like a pass.
#[test]
fn a_truncated_client_hello_is_waited_for_rather_than_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = signing_key(&pki, &rcgen::PKCS_ECDSA_P256_SHA256);
    let config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
    };

    let whole = client_hello(Edit::default());

    // Deliver it a few octets at a time. Every prefix must be accepted
    // silently, and the whole thing must then work.
    let mut server = ServerHandshake::new(&config);
    let body = &whole[5..];
    let mut sent = 0usize;
    let mut reply = Vec::new();
    while sent < body.len() {
        let end = (sent + 40).min(body.len());
        let chunk = &body[sent..end];
        let mut record = vec![22u8, 0x03, 0x01];
        record.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        record.extend_from_slice(chunk);

        let out = server
            .read_record(&record)
            .unwrap_or_else(|e| panic!("a partial ClientHello was refused at {sent}: {e}"));
        if end < body.len() {
            assert!(out.is_empty(), "the server answered a partial ClientHello");
        } else {
            reply = out;
        }
        sent = end;
    }

    assert!(
        !reply.is_empty(),
        "the reassembled ClientHello produced no answer"
    );
    assert_eq!(reply[0], 22, "the answer should start with a ServerHello");
}

/// A peer cannot drive a server by never completing a message.
///
/// The mirror of the client's own version. A header claiming three megabytes,
/// followed by nothing that completes it, must never look like progress.
#[test]
fn a_peer_cannot_drive_the_server_with_incomplete_messages() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = signing_key(&pki, &rcgen::PKCS_ECDSA_P256_SHA256);
    let config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519],
    };
    let mut server = ServerHandshake::new(&config);

    let mut body = vec![0x01u8, 0x30, 0x00, 0x00]; // ClientHello, 0x300000 long
    body.extend_from_slice(&[0u8; 60]);
    let mut record = vec![22u8, 0x03, 0x01];
    record.extend_from_slice(&(body.len() as u16).to_be_bytes());
    record.extend_from_slice(&body);

    for _ in 0..64 {
        let out = server.read_record(&record).expect("buffered");
        assert!(out.is_empty(), "an incomplete message produced an answer");
        assert!(
            !server.is_finished(),
            "an incomplete message finished a handshake"
        );
    }
}

/// A minimal client that controls its own keys, so it can produce a *validly
/// encrypted* Finished carrying the wrong `verify_data`.
///
/// This exists because the first version of the test below corrupted a byte of
/// the protected record instead, which fails at the AEAD layer before the
/// Finished check runs — so a mutation deleting that check survived the whole
/// suite. Corrupting the ciphertext tests the record layer; corrupting the
/// `verify_data` tests the thing that actually matters.
struct TestClient {
    kx: rusty_tls::handrolled::kx::KeyExchange,
    hello: Vec<u8>,
}

impl TestClient {
    fn new() -> Self {
        use rusty_tls::handrolled::handshake::extension;
        use rusty_tls::handrolled::wire::Writer;

        let kx =
            rusty_tls::handrolled::kx::KeyExchange::generate(NamedGroup::X25519).expect("generate");

        let mut share = Writer::new();
        share.vector_u16(|w| {
            w.u16(NamedGroup::X25519.as_u16());
            w.vector_u16(|w| w.bytes(kx.public_key()));
        });
        let mut versions = Writer::new();
        versions.vector_u8(|w| w.u16(0x0304));
        let mut groups = Writer::new();
        groups.vector_u16(|w| w.u16(NamedGroup::X25519.as_u16()));
        let mut schemes = Writer::new();
        schemes.vector_u16(|w| {
            for scheme in rusty_tls::handrolled::verify::SignatureScheme::TLS13_SUPPORTED {
                w.u16(scheme.0);
            }
        });
        let (share, versions, groups, schemes) = (
            share.into_vec(),
            versions.into_vec(),
            groups.into_vec(),
            schemes.into_vec(),
        );

        let hello = ClientHello {
            random: &[0x11u8; 32],
            session_id: &[0x22u8; 32],
            cipher_suites: vec![CipherSuite::TLS_AES_128_GCM_SHA256.0],
            extensions: vec![
                rusty_tls::handrolled::handshake::Extension {
                    typ: extension::SUPPORTED_VERSIONS,
                    data: &versions,
                },
                rusty_tls::handrolled::handshake::Extension {
                    typ: extension::SUPPORTED_GROUPS,
                    data: &groups,
                },
                rusty_tls::handrolled::handshake::Extension {
                    typ: extension::SIGNATURE_ALGORITHMS,
                    data: &schemes,
                },
                rusty_tls::handrolled::handshake::Extension {
                    typ: extension::KEY_SHARE,
                    data: &share,
                },
            ],
        };

        Self {
            kx,
            hello: Message::encode(HandshakeType::ClientHello, &hello.encode()),
        }
    }

    fn hello_record(&self) -> Vec<u8> {
        let mut out = vec![22u8, 0x03, 0x01];
        out.extend_from_slice(&(self.hello.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.hello);
        out
    }

    /// Answer the server's flight with a Finished, optionally a wrong one.
    fn finished(self, server_records: &[Vec<u8>], corrupt: bool) -> Vec<u8> {
        use rusty_tls::handrolled::handshake::{extension, find, Transcript};
        use rusty_tls::handrolled::record::{Aead, ContentType, Opener, Sealer};
        use rusty_tls::handrolled::schedule::{
            finished_verify_data, traffic_keys, Hash, KeySchedule,
        };
        use rusty_tls::handrolled::wire::Reader;

        let (aead, hash) = (Aead::Aes128Gcm, Hash::Sha256);

        let server_hello_record = &server_records[0];
        let parsed = messages(&server_hello_record[5..]).expect("ServerHello parses");
        let server_hello =
            rusty_tls::handrolled::handshake::ServerHello::parse(parsed[0].body).expect("parses");

        let share = find(&server_hello.extensions, extension::KEY_SHARE).expect("key_share");
        let mut reader = Reader::new(share);
        let _group = reader.u16().expect("group");
        let peer_key = reader.vector_u16().expect("key").to_vec();

        let mut transcript = Transcript::new(hash);
        transcript.add(&self.hello);
        transcript.add(parsed[0].encoded);
        let hello_hash = transcript.hash();

        let schedule = self
            .kx
            .agree(&peer_key, |secret| {
                KeySchedule::new(hash).into_handshake(secret)
            })
            .expect("agree");
        let client_secret = schedule.derive("c hs traffic", &hello_hash);
        let server_secret = schedule.derive("s hs traffic", &hello_hash);

        // Open the server's flight so the transcript matches what it sent.
        let keys = traffic_keys(hash, &server_secret, aead.key_len());
        let mut opener = Opener::new(aead, &keys.key, &keys.iv).expect("opener");
        for record in &server_records[1..] {
            if record[0] == 20 {
                continue; // change_cipher_spec
            }
            let opened = opener.open(record).expect("the server's flight opens");
            transcript.add(&opened.fragment);
        }

        let mut verify_data = finished_verify_data(hash, &client_secret, &transcript.hash());
        if corrupt {
            verify_data[0] ^= 0x01;
        }
        let finished = Message::encode(HandshakeType::Finished, &verify_data);

        let keys = traffic_keys(hash, &client_secret, aead.key_len());
        let mut sealer = Sealer::new(aead, &keys.key, &keys.iv).expect("sealer");
        sealer
            .seal(ContentType::Handshake, &finished, 0)
            .expect("seal")
    }
}

/// Drive a [`TestClient`] against this server, and report what happened.
fn against_test_client(corrupt_finished: bool) -> Result<(), ServerError> {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = signing_key(&pki, &rcgen::PKCS_ECDSA_P256_SHA256);
    let config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: &[CipherSuite::TLS_AES_128_GCM_SHA256],
        groups: &[NamedGroup::X25519],
    };
    let mut server = ServerHandshake::new(&config);
    let client = TestClient::new();

    let mut flight = server.read_record(&client.hello_record())?;
    let records = take_records(&mut flight);
    let finished = client.finished(&records, corrupt_finished);
    server.read_record(&finished)?;

    if server.is_finished() {
        Ok(())
    } else {
        Err(ServerError::Failed)
    }
}

/// The control: an honest Finished completes, or the test below would pass
/// for the wrong reason.
#[test]
fn the_test_clients_correct_finished_completes() {
    against_test_client(false).expect("the control handshake must complete");
}

/// A client Finished that does not verify ends the handshake.
///
/// The single most important check a server makes: it is the only thing
/// proving the peer derived the same keys over the same transcript. Without
/// it, a server completes handshakes with anyone who can replay a ClientHello
/// and produce any Finished at all.
///
/// A mutation deleting this check survived the first version of this test,
/// which corrupted a byte of the protected record — that fails at the AEAD
/// layer, several steps earlier, so the check was never reached. The Finished
/// here is correctly encrypted and carries the wrong `verify_data`, which is
/// the only shape that exercises it.
#[test]
fn a_client_finished_that_does_not_verify_is_refused() {
    let error = against_test_client(true).expect_err("a wrong Finished was accepted");
    assert_eq!(error, ServerError::BadFinished);
    assert_eq!(error.alert(), Some(AlertDescription::DECRYPT_ERROR));
}

/// A corrupted record fails at the record layer, which is a different failure
/// from a wrong Finished and is worth keeping distinct.
#[test]
fn a_corrupted_protected_record_fails_at_the_record_layer() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = signing_key(&pki, &rcgen::PKCS_ECDSA_P256_SHA256);
    let config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: &[CipherSuite::TLS_AES_128_GCM_SHA256],
        groups: &[NamedGroup::X25519],
    };
    let mut server = ServerHandshake::new(&config);
    let client = TestClient::new();

    let mut flight = server.read_record(&client.hello_record()).expect("flight");
    let records = take_records(&mut flight);
    let mut finished = client.finished(&records, false);
    let last = finished.len() - 1;
    finished[last] ^= 0x01;

    let error = server
        .read_record(&finished)
        .expect_err("a corrupted record was accepted");
    assert!(
        matches!(error, ServerError::Record(_)),
        "expected a record-layer failure, got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// Signing keys
// ---------------------------------------------------------------------------

/// A key refuses to sign with a scheme it cannot produce, rather than
/// substituting one it can.
#[test]
fn a_signing_key_refuses_a_scheme_it_cannot_produce() {
    use rusty_tls::handrolled::sign::SignError;
    use rusty_tls::handrolled::verify::SignatureScheme;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = SigningKey::ecdsa_p256(&pki.leaf_pkcs8).expect("key");

    assert_eq!(key.schemes(), &[SignatureScheme::ECDSA_SECP256R1_SHA256]);
    for scheme in [
        SignatureScheme::ECDSA_SECP384R1_SHA384,
        SignatureScheme::ED25519,
        SignatureScheme::RSA_PSS_RSAE_SHA256,
        SignatureScheme::RSA_PKCS1_SHA256,
    ] {
        assert_eq!(
            key.sign(scheme, b"anything"),
            Err(SignError::UnsupportedScheme(scheme)),
            "{scheme:?} was signed by a P-256 key"
        );
    }
}

/// A key of the wrong kind is refused at load, not at handshake time.
#[test]
fn a_key_of_the_wrong_kind_is_refused_when_it_is_loaded() {
    use rusty_tls::handrolled::sign::SignError;

    let p256 = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    assert_eq!(
        SigningKey::ecdsa_p384(&p256.leaf_pkcs8).err(),
        Some(SignError::BadKey)
    );
    assert_eq!(
        SigningKey::rsa(&p256.leaf_pkcs8).err(),
        Some(SignError::BadKey)
    );
    assert_eq!(
        SigningKey::ed25519(&p256.leaf_pkcs8).err(),
        Some(SignError::BadKey)
    );
}

/// A signature verifies under the certificate's own key, and only over the
/// bytes that were signed.
#[test]
fn a_signature_verifies_under_the_matching_certificate() {
    use rusty_tls::handrolled::verify::{verify_tls13_signature, SignatureScheme, VerifyError};

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = SigningKey::ecdsa_p256(&pki.leaf_pkcs8).expect("key");
    let leaf = Certificate::parse(&pki.chain[0]).expect("leaf parses");
    let spki = leaf.subject_public_key_info();

    let message = b"the bytes a CertificateVerify would cover";
    let signature = key
        .sign(SignatureScheme::ECDSA_SECP256R1_SHA256, message)
        .expect("sign");

    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::ECDSA_SECP256R1_SHA256,
            &spki,
            message,
            &signature
        ),
        Ok(())
    );
    assert_eq!(
        verify_tls13_signature(
            SignatureScheme::ECDSA_SECP256R1_SHA256,
            &spki,
            b"different bytes entirely",
            &signature
        ),
        Err(VerifyError::BadSignature)
    );
}

/// `Debug` must not carry key material.
#[test]
fn a_signing_keys_debug_says_nothing_useful() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256);
    let key = SigningKey::ecdsa_p256(&pki.leaf_pkcs8).expect("key");
    let rendered = format!("{key:?}");

    assert!(rendered.contains("<redacted>"), "{rendered}");
    assert!(
        rendered.contains("ECDSA P-256"),
        "the algorithm is not secret"
    );
    assert!(
        !rendered.contains(&format!("{:?}", key.public_key())),
        "the key bytes were rendered: {rendered}"
    );
}
