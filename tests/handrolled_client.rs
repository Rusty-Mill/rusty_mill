//! The TLS 1.3 client handshake — stage 3c-ii.
//!
//! # Why interop is the test that matters here
//!
//! Every earlier stage had an oracle that was independent of this code: RFC
//! 8448's published bytes, the machine's own trust store, rustls' record
//! layer. A state machine has a better one — **a real server** — and it is
//! better because it checks the thing unit tests structurally cannot.
//!
//! A handshake is a mutual computation. If this client derives the wrong
//! traffic secret, builds the transcript in the wrong order, or encodes an
//! extension slightly wrong, a self-consistent test suite would still pass:
//! both sides of it are this code. `rustls` did not read this implementation,
//! so a completed handshake against it is evidence about the protocol rather
//! than about internal agreement.
//!
//! [`a_full_handshake_against_rustls_completes_and_carries_data`] is therefore
//! the load-bearing test in this file, and the rest exists because interop
//! proves the happy path and says nothing about refusals.
//!
//! # The refusals are the other half
//!
//! A client that completes a handshake with a good server and *also* completes
//! one with an attacker is worse than useless. The tampering tests drive real
//! handshakes and corrupt one thing each — the certificate, the signature, the
//! Finished, the order of the flight — and require every one to be refused.
//!
//! The sharpest is [`a_flight_without_a_certificate_verify_is_refused`]. A
//! Certificate proves nothing on its own; anybody can replay somebody else's.
//! Only the CertificateVerify proves the peer holds the key, so a client that
//! tolerated its absence would authenticate an attacker who copied a
//! certificate off the wire.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{
    record_length, CipherSuite, ClientConfig, ClientError, ClientHandshake, ClientIdentity,
    Incoming, Resumption, Session,
};
use rusty_tls::handrolled::handshake::{
    messages, ClientHello, HandshakeError, HandshakeType, Message, PresharedKeyOffer, ServerHello,
};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::x509::Certificate;

/// The rejection cases, shared with `handshake.rs`. This file holds the
/// hand-rolled driver for them; that one holds the `rustls` driver.
mod rejection;

const SERVER: &str = "handrolled.example";

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

// ---------------------------------------------------------------------------
// A server to talk to
// ---------------------------------------------------------------------------

/// A CA, and a leaf it issued for [`SERVER`].
struct Pki {
    root_der: Vec<u8>,
    leaf_der: Vec<u8>,
    /// The leaf's private key, for the test server below to sign with.
    leaf_pkcs8: Vec<u8>,
    chain: Vec<CertificateDer<'static>>,
    /// The same chain as plain DER, which is the shape this crate's own
    /// `ClientIdentity` takes.
    chain_der: Vec<Vec<u8>>,
    key: PrivateKeyDer<'static>,
}

/// An explicit validity window, because `rcgen` defaults `not_after` to the
/// year 4096 and a test that relies on that default cannot express "expired".
const NOT_BEFORE: i64 = 1_577_836_800; // 2020-01-01
const NOT_AFTER: i64 = 1_893_456_000; // 2030-01-01

fn dated(params: &mut CertificateParams) {
    params.not_before = OffsetDateTime::from_unix_timestamp(NOT_BEFORE).expect("not_before");
    params.not_after = OffsetDateTime::from_unix_timestamp(NOT_AFTER).expect("not_after");
}

fn pki(algorithm: &'static rcgen::SignatureAlgorithm, name: &str) -> Pki {
    let root_key = KeyPair::generate_for(algorithm).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("root params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    root_params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("handrolled test root".to_string()),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");

    let leaf_key = KeyPair::generate_for(algorithm).expect("leaf key");
    let mut leaf_params = CertificateParams::new(vec![name.to_string()]).expect("leaf params");
    dated(&mut leaf_params);
    let leaf = leaf_params
        .signed_by(&leaf_key, &root, &root_key)
        .expect("leaf");

    Pki {
        root_der: root.der().to_vec(),
        leaf_der: leaf.der().to_vec(),
        leaf_pkcs8: leaf_key.serialize_der(),
        chain: vec![
            CertificateDer::from(leaf.der().to_vec()),
            CertificateDer::from(root.der().to_vec()),
        ],
        chain_der: vec![leaf.der().to_vec(), root.der().to_vec()],
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    }
}

fn anchor(root_der: &[u8]) -> Certificate<'_> {
    Certificate::parse(root_der).expect("the root parses")
}

fn rustls_server(pki: &Pki) -> rustls::ServerConnection {
    rustls_server_presenting(pki.chain.clone(), pki.key.clone_key())
}

/// As [`rustls_server`], but for a chain that did not come from [`Pki`] — the
/// shared rejection table builds its own.
fn rustls_server_presenting(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> rustls::ServerConnection {
    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .expect("server config");
    rustls::ServerConnection::new(Arc::new(config)).expect("server connection")
}

// ---------------------------------------------------------------------------
// Driving the two against each other
// ---------------------------------------------------------------------------

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

/// Feed bytes to a rustls server and collect whatever it wants to send back.
fn pump_server(server: &mut rustls::ServerConnection, input: &[u8]) -> Vec<u8> {
    if !input.is_empty() {
        let mut cursor = std::io::Cursor::new(input);
        while server.read_tls(&mut cursor).expect("read_tls") > 0 {
            server.process_new_packets().expect("process_new_packets");
        }
    }
    server.process_new_packets().expect("process_new_packets");
    let mut out = Vec::new();
    while server.wants_write() {
        server.write_tls(&mut out).expect("write_tls");
    }
    out
}

/// As [`pump_server`], but tolerating a server that refuses the handshake.
///
/// `process_new_packets` returns an error when `rustls` rejects a ClientHello
/// — which is the *correct* behaviour for a TLS 1.2-only server faced with a
/// client offering only 1.3. It still has an alert queued, and that alert is
/// the whole point of the test, so the error is reported rather than asserted
/// away.
fn pump_server_allowing_refusal(
    server: &mut rustls::ServerConnection,
    input: &[u8],
) -> (Vec<u8>, Option<String>) {
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

/// What a completed handshake produced, so tests can keep talking.
struct Established {
    connection: rusty_tls::handrolled::client::Connection,
    server: rustls::ServerConnection,
}

/// Run a handshake to completion, optionally corrupting the server's records
/// on the way through.
///
/// `tamper` sees every record the server sends, in order, and returns what the
/// client should actually receive. Returning `None` drops the record.
fn handshake_with(
    pki: &Pki,
    name: &str,
    mut tamper: impl FnMut(usize, Vec<u8>) -> Option<Vec<u8>>,
) -> Result<Established, ClientError> {
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let config = ClientConfig {
        server_name: ServerName::Dns(name),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::X25519, NamedGroup::SecP256R1],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };

    let mut server = rustls_server(pki);
    let (mut client, mut to_server) = ClientHandshake::start(&config)?;

    let mut seen = 0usize;
    for _ in 0..16 {
        let from_server = pump_server(&mut server, &to_server);
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
            // Deliver the client's final flight, plus anything the server
            // says afterwards (rustls sends session tickets immediately).
            let mut connection = client.into_connection()?;
            let tickets = pump_server(&mut server, &to_server);
            let mut stream = tickets;
            for record in take_records(&mut stream) {
                // Asserted here rather than in one dedicated test, because
                // this is where every completed handshake passes and a
                // mutation that surfaced a ticket as data survived a suite
                // that only checked it in one place.
                if let Incoming::Application(data) = connection.read(&record)? {
                    panic!("a post-handshake message surfaced as data: {data:02x?}");
                }
            }
            return Ok(Established { connection, server });
        }
        if to_server.is_empty() {
            break;
        }
    }

    Err(ClientError::Failed)
}

fn handshake(pki: &Pki) -> Result<Established, ClientError> {
    handshake_with(pki, SERVER, |_, record| Some(record))
}

// ---------------------------------------------------------------------------
// Interop — the test that carries this file
// ---------------------------------------------------------------------------

/// A complete handshake against a real `rustls` server, then data both ways.
///
/// See the module docs on why this is worth more than any number of
/// self-consistent unit tests: rustls has not read this implementation, so
/// agreement is evidence about TLS rather than about internal consistency.
#[test]
fn a_full_handshake_against_rustls_completes_and_carries_data() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let Established {
        mut connection,
        mut server,
    } = handshake(&pki).expect("the handshake completes");

    // The client saw the chain the server actually sent.
    assert_eq!(connection.peer_certificates().len(), 2);
    let leaf = Certificate::parse(&connection.peer_certificates()[0]).expect("parses");
    assert_eq!(leaf.subject_public_key_info().key.len(), 65, "a P-256 key");

    // Client to server.
    let record = connection
        .write(b"ping from the hand-rolled client")
        .expect("write");
    let mut cursor = std::io::Cursor::new(&record);
    server.read_tls(&mut cursor).expect("read_tls");
    server.process_new_packets().expect("process");
    let mut got = Vec::new();
    std::io::Read::read_to_end(&mut server.reader(), &mut got).ok();
    assert_eq!(got, b"ping from the hand-rolled client");

    // Server to client.
    std::io::Write::write_all(&mut server.writer(), b"pong from rustls").expect("write");
    let mut out = Vec::new();
    while server.wants_write() {
        server.write_tls(&mut out).expect("write_tls");
    }
    let mut received = Vec::new();
    for record in take_records(&mut out) {
        if let Incoming::Application(data) = connection.read(&record).expect("read") {
            received.extend_from_slice(&data);
        }
    }
    assert_eq!(received, b"pong from rustls");
}

/// Every cipher suite and both key-exchange groups, against a real server.
///
/// A suite that is offered but broken is worse than one that is absent: the
/// server picks it and the handshake fails for a reason nobody can see.
#[test]
fn every_offered_suite_and_group_completes_against_rustls() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];

    for suite in CipherSuite::SUPPORTED {
        for group in [
            NamedGroup::X25519,
            NamedGroup::SecP256R1,
            NamedGroup::SecP384R1,
        ] {
            let config = ClientConfig {
                server_name: ServerName::Dns(SERVER),
                anchors: &anchors,
                path: options(),
                groups: &[group],
                cipher_suites: core::slice::from_ref(suite),
                identity: None,
                resumption: None,
            };

            let mut server = rustls_server(&pki);
            let (mut client, mut to_server) = ClientHandshake::start(&config).expect("start");

            for _ in 0..8 {
                let mut stream = pump_server(&mut server, &to_server);
                to_server.clear();
                for record in take_records(&mut stream) {
                    to_server.extend_from_slice(
                        &client
                            .read_record(&record)
                            .unwrap_or_else(|e| panic!("{suite:?} {group:?}: {e}")),
                    );
                }
                if client.is_finished() {
                    break;
                }
            }

            let connection = client
                .into_connection()
                .unwrap_or_else(|e| panic!("{suite:?} {group:?} did not complete: {e}"));
            assert_eq!(connection.cipher_suite(), *suite);
        }
    }
}

// ---------------------------------------------------------------------------
// What the client sends
// ---------------------------------------------------------------------------

/// The ClientHello has to be the message TLS 1.3 requires, and a server that
/// rejects it says so only by failing the handshake — which is a poor error
/// message. This checks the shape directly.
#[test]
fn the_client_hello_offers_what_it_should_and_nothing_it_should_not() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let config = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::X25519, NamedGroup::SecP256R1],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };

    let (_, record) = ClientHandshake::start(&config).expect("start");
    assert_eq!(record[0], 22, "a handshake record");
    let parsed = messages(&record[5..]).expect("the body is one message");
    assert_eq!(parsed[0].typ, HandshakeType::ClientHello);

    let hello = ClientHello::parse(parsed[0].body).expect("parses");
    assert_eq!(hello.random.len(), 32);
    // RFC 8446 §D.4: a non-empty session id, for middleboxes.
    assert_eq!(hello.session_id.len(), 32);
    assert_eq!(
        hello.cipher_suites,
        CipherSuite::SUPPORTED
            .iter()
            .map(|s| s.0)
            .collect::<Vec<_>>()
    );

    use rusty_tls::handrolled::handshake::{extension, find};
    assert_eq!(
        find(&hello.extensions, extension::SUPPORTED_VERSIONS),
        Some(&[0x02, 0x03, 0x04][..]),
        "exactly TLS 1.3, and nothing older"
    );
    let sni = find(&hello.extensions, extension::SERVER_NAME).expect("SNI is present");
    assert!(sni.ends_with(SERVER.as_bytes()));
    assert!(find(&hello.extensions, extension::KEY_SHARE).is_some());
    assert!(find(&hello.extensions, extension::SIGNATURE_ALGORITHMS).is_some());
    assert!(find(&hello.extensions, extension::SUPPORTED_GROUPS).is_some());
}

/// RFC 6066 §3: an IP address is never sent as a `server_name`. Sending one
/// leaks the address to anything reading the plaintext ClientHello and is not
/// what the extension means.
#[test]
fn an_ip_address_is_not_sent_as_a_server_name() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let config = ClientConfig {
        server_name: ServerName::Ip("192.0.2.1".parse().expect("address")),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };

    let (_, record) = ClientHandshake::start(&config).expect("start");
    let parsed = messages(&record[5..]).expect("parses");
    let hello = ClientHello::parse(parsed[0].body).expect("parses");

    use rusty_tls::handrolled::handshake::{extension, find};
    assert_eq!(
        find(&hello.extensions, extension::SERVER_NAME),
        None,
        "an IP address was sent as SNI"
    );
    assert!(
        !record.windows(9).any(|w| w == b"192.0.2.1"),
        "the address appears in the ClientHello anyway"
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// The control: the test server's correct flight must complete, or every
/// refusal below would pass for the wrong reason.
///
/// Without this, a test server that produced garbage would make the whole
/// rejection suite green while proving nothing at all.
#[test]
fn the_test_servers_correct_flight_completes() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    against_test_server(&pki, Shape::Correct).expect("the control handshake must complete");
}

/// The sharpest refusal in the file. See the module docs.
///
/// The certificate is genuine and chains to a trusted root. What is missing is
/// the only thing that proves the peer holds its private key, and a client that
/// shrugged would authenticate anyone who copied a certificate off the wire.
#[test]
fn a_flight_without_a_certificate_verify_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let error = against_test_server(&pki, Shape::Missing(HandshakeType::CertificateVerify))
        .expect_err("a flight with no CertificateVerify completed");
    assert!(
        matches!(
            error,
            ClientError::UnexpectedMessage {
                expected: "CertificateVerify",
                got: HandshakeType::Finished,
            }
        ),
        "refused for the wrong reason: {error:?}"
    );
}

/// Every message in the server's flight is required, and in order.
#[test]
fn a_flight_missing_any_message_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);

    for (dropped, expected) in [
        (HandshakeType::EncryptedExtensions, "EncryptedExtensions"),
        (HandshakeType::Certificate, "Certificate"),
        (HandshakeType::CertificateVerify, "CertificateVerify"),
        (HandshakeType::Finished, "Finished"),
    ] {
        let outcome = against_test_server(&pki, Shape::Missing(dropped));
        match outcome {
            Err(ClientError::UnexpectedMessage { expected: e, .. }) => assert_eq!(
                e, expected,
                "dropping {dropped:?} was refused as a missing {e}, not {expected}"
            ),
            // Dropping the Finished leaves the handshake unfinished rather
            // than producing a message to object to, which is equally a
            // refusal: nothing completes.
            Err(ClientError::Failed) if dropped == HandshakeType::Finished => {}
            other => panic!("dropping {dropped:?} was not refused: {other:?}"),
        }
    }
}

/// Order is part of the requirement, not a convention. A CertificateVerify
/// that arrives before the certificate it is about cannot have signed a
/// transcript containing it.
#[test]
fn a_reordered_flight_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let error = against_test_server(&pki, Shape::CertificateVerifyBeforeCertificate)
        .expect_err("a reordered flight completed");
    assert!(
        matches!(
            error,
            ClientError::UnexpectedMessage {
                expected: "Certificate",
                got: HandshakeType::CertificateVerify,
            }
        ),
        "refused for the wrong reason: {error:?}"
    );
}

/// A CertificateVerify signature over anything other than the transcript
/// through the Certificate is refused.
///
/// This is the check that ties the peer's key to *this* handshake rather than
/// to some other one. A signature that verifies over the wrong bytes would let
/// a recorded handshake be replayed.
#[test]
fn a_signature_over_the_wrong_transcript_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let error = against_test_server(&pki, Shape::SignatureOverTheWrongTranscript)
        .expect_err("a signature over the wrong bytes was accepted");
    assert!(
        matches!(error, ClientError::Verify(_)),
        "refused for the wrong reason: {error:?}"
    );
}

/// A Finished that does not verify ends the handshake. It is the only thing
/// that proves the two sides derived the same keys over the same transcript.
#[test]
fn a_finished_that_does_not_verify_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let error = against_test_server(&pki, Shape::CorruptFinished)
        .expect_err("a corrupt Finished was accepted");
    assert_eq!(error, ClientError::BadFinished);
}

/// A CertificateRequest with no `signature_algorithms` is refused.
///
/// RFC 8446 §4.3.2 makes the extension mandatory, and for a reason worth
/// keeping: without it the request names no scheme, so any answer would be a
/// guess. A guess produces a signature the server is obliged to reject, and a
/// failure two messages later that looks like a signing bug.
#[test]
fn a_certificate_request_without_signature_algorithms_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let error = against_test_server(&pki, Shape::RequestClientCertificate)
        .expect_err("a CertificateRequest naming no schemes was answered anyway");
    assert_eq!(
        error,
        ClientError::Handshake(HandshakeError::MissingSignatureAlgorithms)
    );
}

/// A minimal TLS 1.3 server, built from this crate's own primitives.
///
/// It exists because the server's flight arrives inside one AEAD-protected
/// record: editing it on the wire needs keys the test does not have, so the
/// only way to show the client a malformed flight is to be the peer that
/// produces one.
///
/// Using this crate's own primitives to test this crate would be circular if
/// it were proving the client *works* — which is why it is not used for that.
/// The happy path is proved against `rustls` above. This is used only to make
/// the client **refuse**, and a refusal cannot be manufactured by shared
/// wrongness: if both sides agreed on a wrong flight, the client would accept
/// it and the test would fail.
struct TestServer<'a> {
    pki: &'a Pki,
    shape: Shape,
    /// Seal the flight in chunks of this many octets instead of one record.
    ///
    /// Handshake messages may span records and several may share one, so the
    /// two boundaries have nothing to do with each other. Neither `rustls`
    /// nor any real server tried in `handrolled_interop` actually splits a
    /// flight — so without this, the client's reassembly buffer was carried
    /// by tests that never made it do anything.
    fragment: Option<usize>,
}

/// How the flight should be malformed, if at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// Everything, in order — the control, which must complete.
    Correct,
    /// Omit one message.
    Missing(HandshakeType),
    /// Send Certificate and CertificateVerify the wrong way round.
    CertificateVerifyBeforeCertificate,
    /// A CertificateVerify whose signature is over the wrong transcript.
    SignatureOverTheWrongTranscript,
    /// A Finished whose `verify_data` is one bit out.
    CorruptFinished,
    /// A CertificateRequest, which this client refuses.
    RequestClientCertificate,
    /// A `key_share` naming a group other than the one the client sent.
    WrongKeyShareGroup,
    /// A fatal alert where the encrypted flight should be — a server changing
    /// its mind after the ServerHello, which is where a real one would report
    /// that it disliked something about the client.
    AlertInsteadOfFlight,
}

impl TestServer<'_> {
    /// Answer a ClientHello record with a flight of the configured shape,
    /// returning the records to send back and the server's own
    /// application-direction state.
    ///
    /// The application secrets come from the transcript through the *server's*
    /// Finished, so the server can compute them without ever seeing the
    /// client's — which is what makes post-handshake testing possible here.
    fn respond(&self, client_hello_record: &[u8]) -> (Vec<u8>, ServerPost) {
        use rusty_tls::handrolled::handshake::{
            certificate_verify_content, extension, find, CertificateEntry, CertificateMessage,
            CertificateVerify, Extension, Transcript, SERVER_CERTIFICATE_VERIFY_CONTEXT,
        };
        use rusty_tls::handrolled::kx::{KeyExchange, NamedGroup};
        use rusty_tls::handrolled::record::{Aead, ContentType, Sealer};
        use rusty_tls::handrolled::schedule::{
            finished_verify_data, traffic_keys, Hash, KeySchedule,
        };
        use rusty_tls::handrolled::wire::{Reader, Writer};

        let (aead, hash) = (Aead::Aes128Gcm, Hash::Sha256);

        let parsed = messages(&client_hello_record[5..]).expect("the ClientHello parses");
        let hello = ClientHello::parse(parsed[0].body).expect("parses");

        // The client's single key_share entry.
        let shares = find(&hello.extensions, extension::KEY_SHARE).expect("a key_share");
        let mut reader = Reader::new(shares);
        let mut list = reader.sub_u16().expect("client_shares");
        let group = NamedGroup::from_u16(list.u16().expect("group")).expect("a known group");
        let peer_key = list.vector_u16().expect("key").to_vec();

        let kx = KeyExchange::generate(group).expect("generate");
        let public = kx.public_key().to_vec();

        let mut share = Writer::new();
        share.u16(if self.shape == Shape::WrongKeyShareGroup {
            // A well-formed X25519 key, labelled P-256. The bytes are usable;
            // only the label is a lie.
            NamedGroup::SecP256R1.as_u16()
        } else {
            group.as_u16()
        });
        share.vector_u16(|w| w.bytes(&public));
        let share = share.into_vec();
        let versions = vec![0x03, 0x04];

        let server_hello = ServerHello {
            random: &[0x5au8; 32],
            session_id: hello.session_id,
            cipher_suite: CipherSuite::TLS_AES_128_GCM_SHA256.0,
            extensions: vec![
                Extension {
                    typ: extension::KEY_SHARE,
                    data: &share,
                },
                Extension {
                    typ: extension::SUPPORTED_VERSIONS,
                    data: &versions,
                },
            ],
        };
        let server_hello = Message::encode(HandshakeType::ServerHello, &server_hello.encode());

        let mut transcript = Transcript::new(hash);
        transcript.add(parsed[0].encoded);
        transcript.add(&server_hello);

        let schedule = kx
            .agree(&peer_key, |secret| {
                KeySchedule::new(hash).into_handshake(secret)
            })
            .expect("agree");
        let server_secret = schedule.derive("s hs traffic", &transcript.hash());
        let keys = traffic_keys(hash, &server_secret, aead.key_len());
        let mut sealer = Sealer::new(aead, &keys.key, &keys.iv).expect("sealer");

        // Build the flight, adding each message to the transcript as it is
        // actually sent — so a dropped message is genuinely absent from both.
        let mut flight = Vec::new();
        let add = |transcript: &mut Transcript, flight: &mut Vec<u8>, bytes: &[u8]| {
            transcript.add(bytes);
            flight.extend_from_slice(bytes);
        };

        let mut empty_extensions = Writer::new();
        empty_extensions.vector_u16(|_| {});
        let encrypted_extensions = Message::encode(
            HandshakeType::EncryptedExtensions,
            &empty_extensions.into_vec(),
        );

        let certificate = CertificateMessage {
            context: &[],
            entries: vec![
                CertificateEntry {
                    certificate: &self.pki.leaf_der,
                    extensions: &[],
                },
                CertificateEntry {
                    certificate: &self.pki.root_der,
                    extensions: &[],
                },
            ],
        };
        let certificate = Message::encode(HandshakeType::Certificate, &certificate.encode());

        if self.shape != Shape::Missing(HandshakeType::EncryptedExtensions) {
            add(&mut transcript, &mut flight, &encrypted_extensions);
        }
        if self.shape == Shape::RequestClientCertificate {
            let mut body = Writer::new();
            body.vector_u8(|_| {}); // certificate_request_context
            body.vector_u16(|_| {}); // extensions
            let request = Message::encode(HandshakeType::CertificateRequest, &body.into_vec());
            add(&mut transcript, &mut flight, &request);
        }

        let sign = |transcript_hash: &[u8]| -> Vec<u8> {
            let content =
                certificate_verify_content(SERVER_CERTIFICATE_VERIFY_CONTEXT, transcript_hash);
            let rng = ring::rand::SystemRandom::new();
            let pair = ring::signature::EcdsaKeyPair::from_pkcs8(
                &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
                &self.pki.leaf_pkcs8,
                &rng,
            )
            .expect("the leaf key loads");
            let signature = pair.sign(&rng, &content).expect("sign").as_ref().to_vec();
            let verify = CertificateVerify {
                scheme: 0x0403, // ecdsa_secp256r1_sha256
                signature: &signature,
            };
            Message::encode(HandshakeType::CertificateVerify, &verify.encode())
        };

        match self.shape {
            Shape::CertificateVerifyBeforeCertificate => {
                let verify = sign(&transcript.hash());
                add(&mut transcript, &mut flight, &verify);
                add(&mut transcript, &mut flight, &certificate);
            }
            Shape::Missing(HandshakeType::Certificate) => {
                let verify = sign(&transcript.hash());
                add(&mut transcript, &mut flight, &verify);
            }
            Shape::Missing(HandshakeType::CertificateVerify) => {
                add(&mut transcript, &mut flight, &certificate);
            }
            Shape::SignatureOverTheWrongTranscript => {
                add(&mut transcript, &mut flight, &certificate);
                // Sign the transcript as it will be *after* this message
                // rather than before it — a plausible off-by-one, and fatal.
                let verify = sign(&hash.hash(b"not the transcript"));
                add(&mut transcript, &mut flight, &verify);
            }
            _ => {
                add(&mut transcript, &mut flight, &certificate);
                let verify = sign(&transcript.hash());
                add(&mut transcript, &mut flight, &verify);
            }
        }

        if self.shape != Shape::Missing(HandshakeType::Finished) {
            let mut verify_data = finished_verify_data(hash, &server_secret, &transcript.hash());
            if self.shape == Shape::CorruptFinished {
                verify_data[0] ^= 0x01;
            }
            let finished = Message::encode(HandshakeType::Finished, &verify_data);
            add(&mut transcript, &mut flight, &finished);
        }

        let mut out = Vec::new();
        out.extend_from_slice(&plaintext(22, &server_hello));

        if self.shape == Shape::AlertInsteadOfFlight {
            out.extend_from_slice(
                &sealer
                    .seal(ContentType::Alert, &[0x02, 0x28], 0) // fatal, handshake_failure
                    .expect("seal"),
            );
            let master = schedule.into_master();
            let secret = master.derive("s ap traffic", &transcript.hash());
            let keys = traffic_keys(hash, &secret, aead.key_len());
            return (
                out,
                ServerPost {
                    aead,
                    hash,
                    secret,
                    sealer: Sealer::new(aead, &keys.key, &keys.iv).expect("app sealer"),
                },
            );
        }

        match self.fragment {
            None => out.extend_from_slice(
                &sealer
                    .seal(ContentType::Handshake, &flight, 0)
                    .expect("seal"),
            ),
            Some(size) => {
                for chunk in flight.chunks(size.max(1)) {
                    out.extend_from_slice(
                        &sealer.seal(ContentType::Handshake, chunk, 0).expect("seal"),
                    );
                }
            }
        }

        let master = schedule.into_master();
        let secret = master.derive("s ap traffic", &transcript.hash());
        let keys = traffic_keys(hash, &secret, aead.key_len());
        let post = ServerPost {
            aead,
            hash,
            secret,
            sealer: Sealer::new(aead, &keys.key, &keys.iv).expect("app sealer"),
        };
        (out, post)
    }
}

/// The server's application-direction state, for talking after the handshake.
struct ServerPost {
    aead: rusty_tls::handrolled::record::Aead,
    hash: rusty_tls::handrolled::schedule::Hash,
    secret: Vec<u8>,
    sealer: rusty_tls::handrolled::record::Sealer,
}

impl ServerPost {
    fn seal(&mut self, typ: rusty_tls::handrolled::record::ContentType, body: &[u8]) -> Vec<u8> {
        self.sealer.seal(typ, body, 0).expect("seal")
    }

    /// Advance this direction's key, as a peer does after sending a KeyUpdate.
    fn rekey(&mut self) {
        use rusty_tls::handrolled::record::Sealer;
        use rusty_tls::handrolled::schedule::{traffic_keys, update_traffic_secret};
        self.secret = update_traffic_secret(self.hash, &self.secret);
        let keys = traffic_keys(self.hash, &self.secret, self.aead.key_len());
        self.sealer = Sealer::new(self.aead, &keys.key, &keys.iv).expect("rekeyed sealer");
    }
}

fn plaintext(typ: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![typ, 0x03, 0x03];
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// Drive the client against a [`TestServer`] of the given shape.
fn against_test_server(pki: &Pki, shape: Shape) -> Result<(), ClientError> {
    established_with_test_server(pki, shape).map(|_| ())
}

/// As [`against_test_server`], keeping both ends so a test can carry on
/// talking after the handshake.
fn established_with_test_server(
    pki: &Pki,
    shape: Shape,
) -> Result<(rusty_tls::handrolled::client::Connection, ServerPost), ClientError> {
    let root = anchor(&pki.root_der);
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
        cipher_suites: &[CipherSuite::TLS_AES_128_GCM_SHA256],
        identity: None,
        resumption: None,
    };

    let (mut client, hello) = ClientHandshake::start(&config)?;
    let server = TestServer {
        pki,
        shape,
        fragment: None,
    };
    let (mut stream, post) = server.respond(&hello);

    for record in take_records(&mut stream) {
        client.read_record(&record)?;
    }
    if client.is_finished() {
        Ok((client.into_connection()?, post))
    } else {
        Err(ClientError::Failed)
    }
}

/// The hand-rolled driver for the shared rejection table.
///
/// This replaces three hand-written tests — untrusted root, wrong name,
/// expired — that had hand-written counterparts in `handshake.rs`. The cases
/// now live in `tests/rejection/mod.rs`, and both engines run the same rows
/// against byte-identical certificates.
///
/// Every case is attempted before anything is asserted, so a table that moves
/// reports every row that moved rather than only the first.
#[test]
fn the_shared_rejection_table_holds_for_the_handrolled_engine() {
    rejection::assert_table_is_coherent();

    let mut failures = Vec::new();

    for case in rejection::CASES {
        let fixture = rejection::fixture(case);
        let root = anchor(&fixture.trusted_root_der);
        let anchors = [TrustAnchor {
            subject: root.subject(),
            public_key: root.subject_public_key_info(),
            name_constraints: None,
        }];
        // Expiry lives in the certificate, not in this clock — see the table's
        // `Validity`. The instant is fixed so a verdict never depends on how
        // long the suite took to run.
        let mut path = options();
        path.time = rejection::EVALUATED_AT;
        let config = ClientConfig {
            server_name: ServerName::Dns(case.requested_name),
            anchors: &anchors,
            path,
            groups: &[NamedGroup::X25519],
            cipher_suites: CipherSuite::SUPPORTED,
            identity: None,
            resumption: None,
        };

        match (run_table_case(&fixture, &config), case.handrolled) {
            (Ok(()), rejection::Outcome::Accepted) => {}
            (Err(_), rejection::Outcome::Rejected) if case.handrolled_reason.is_none() => {
                unreachable!("the table's coherence check forbids this")
            }
            (Err(error), rejection::Outcome::Rejected) => {
                // "Refused" is not enough. A client that turned away a good
                // chain over an unrelated failure would satisfy a bare
                // `is_err()`, and the tests this replaced asserted the
                // variant for exactly that reason.
                let Some(rejection::Reason::PathValidation) = case.handrolled_reason else {
                    unreachable!()
                };
                if !matches!(error, ClientError::Path(_)) {
                    failures.push(format!(
                        "{}: refused, but not by path validation: {error:?}",
                        case.name
                    ));
                }
            }
            (Ok(()), rejection::Outcome::Rejected) => {
                failures.push(format!("{}: accepted, expected a refusal", case.name));
            }
            (Err(error), rejection::Outcome::Accepted) => {
                failures.push(format!("{}: refused a good chain: {error:?}", case.name));
            }
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Drive one table case to a verdict.
///
/// Returns rather than panicking on a completed handshake, because the table
/// has an accepting row — a driver that could only express failure would pass
/// every rejection case even if it never reached the certificate.
fn run_table_case(
    fixture: &rejection::Fixture,
    config: &ClientConfig<'_>,
) -> Result<(), ClientError> {
    let mut server = rustls_server_presenting(fixture.chain.clone(), fixture.key.clone_key());
    let (mut client, mut to_server) = ClientHandshake::start(config)?;

    for _ in 0..16 {
        let mut stream = pump_server(&mut server, &to_server);
        to_server.clear();
        for record in take_records(&mut stream) {
            to_server.extend_from_slice(&client.read_record(&record)?);
        }
        if client.is_finished() {
            return Ok(());
        }
        if to_server.is_empty() {
            break;
        }
    }
    Err(ClientError::Failed)
}

/// A tampered record does not decrypt, and the failure is permanent. A client
/// that carried on after a decryption failure would be continuing with state
/// the peer does not share.
#[test]
fn a_tampered_record_fails_the_connection_permanently() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let mut server = rustls_server(&pki);
    let (mut client, to_server) = ClientHandshake::start(&config).expect("start");
    let mut stream = pump_server(&mut server, &to_server);
    let records = take_records(&mut stream);

    // The ServerHello is plaintext; rustls then sends a change_cipher_spec,
    // which is dropped by design, and then the protected flight. Find the
    // first record that is actually encrypted rather than assuming an index.
    client.read_record(&records[0]).expect("ServerHello");
    let flight = records
        .iter()
        .find(|r| r[0] == 23)
        .expect("an encrypted record")
        .clone();

    let mut tampered = flight.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;

    assert!(
        client.read_record(&tampered).is_err(),
        "a tampered record opened"
    );
    assert_eq!(
        client.read_record(&flight),
        Err(ClientError::Failed),
        "the connection continued after a decryption failure"
    );
}

/// A ServerHello selecting a suite the client never offered is refused. A
/// client that accepted one would be negotiating with itself.
#[test]
fn a_cipher_suite_that_was_not_offered_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    // Offer only AES-128; rewrite the ServerHello to name ChaCha20.
    let config = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::X25519],
        cipher_suites: &[CipherSuite::TLS_AES_128_GCM_SHA256],
        identity: None,
        resumption: None,
    };

    let mut server = rustls_server(&pki);
    let (mut client, to_server) = ClientHandshake::start(&config).expect("start");
    let mut stream = pump_server(&mut server, &to_server);
    let records = take_records(&mut stream);

    let mut hello = records[0].clone();
    let parsed = messages(&hello[5..]).expect("parses");
    let body = ServerHello::parse(parsed[0].body).expect("parses");
    let mut rewritten = body.clone();
    rewritten.cipher_suite = CipherSuite::TLS_CHACHA20_POLY1305_SHA256.0;
    let encoded = Message::encode(HandshakeType::ServerHello, &rewritten.encode());
    hello.truncate(5);
    hello[3..5].copy_from_slice(&(encoded.len() as u16).to_be_bytes());
    hello.extend_from_slice(&encoded);

    assert_eq!(
        client.read_record(&hello),
        Err(ClientError::UnofferedCipherSuite(
            CipherSuite::TLS_CHACHA20_POLY1305_SHA256.0
        ))
    );
}

/// A server that does not select TLS 1.3 is refused, whatever it puts in
/// `legacy_version`. That field is pinned for every version, so the absence of
/// `supported_versions` *is* the downgrade.
#[test]
fn a_server_that_does_not_select_tls13_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let mut server = rustls_server(&pki);
    let (mut client, to_server) = ClientHandshake::start(&config).expect("start");
    let mut stream = pump_server(&mut server, &to_server);
    let records = take_records(&mut stream);

    let parsed = messages(&records[0][5..]).expect("parses");
    let body = ServerHello::parse(parsed[0].body).expect("parses");
    let mut rewritten = body.clone();
    rewritten
        .extensions
        .retain(|e| e.typ != rusty_tls::handrolled::handshake::extension::SUPPORTED_VERSIONS);
    let encoded = Message::encode(HandshakeType::ServerHello, &rewritten.encode());

    let mut record = vec![22u8, 0x03, 0x03];
    record.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
    record.extend_from_slice(&encoded);

    assert_eq!(client.read_record(&record), Err(ClientError::NotTls13));
}

/// A record arriving before the ServerHello that claims to be application data
/// cannot be: there is no key yet.
#[test]
fn protected_data_before_the_server_hello_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let (mut client, _) = ClientHandshake::start(&config).expect("start");
    let record = vec![23u8, 0x03, 0x03, 0x00, 0x02, 0xaa, 0xbb];
    assert!(matches!(
        client.read_record(&record),
        Err(ClientError::UnexpectedContentType(_))
    ));
}

// ---------------------------------------------------------------------------
// Post-handshake
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Deframing
// ---------------------------------------------------------------------------

/// `record_length` reports what it can and admits what it cannot.
#[test]
fn record_length_needs_a_whole_header() {
    assert_eq!(record_length(&[]), None);
    assert_eq!(record_length(&[22, 3, 3, 0]), None);
    assert_eq!(record_length(&[22, 3, 3, 0, 5]), Some(10));
    assert_eq!(record_length(&[22, 3, 3, 0x40, 0x00]), Some(16389));
    // It reports the length the header claims, without judging it — checking
    // is the handshake's job, and a deframer that also judged would be two
    // things.
    assert_eq!(record_length(&[99, 9, 9, 0xff, 0xff]), Some(65540));
}

// ---------------------------------------------------------------------------
// HelloRetryRequest
// ---------------------------------------------------------------------------

/// A `rustls` server that will only do X25519, so a client offering a
/// key share for anything else is sent a HelloRetryRequest.
fn x25519_only_server(pki: &Pki) -> rustls::ServerConnection {
    let mut provider = rustls::crypto::ring::default_provider();
    provider.kx_groups = vec![rustls::crypto::ring::kx_group::X25519];

    let config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(pki.chain.clone(), pki.key.clone_key())
        .expect("server config");
    rustls::ServerConnection::new(Arc::new(config)).expect("server connection")
}

/// A real HelloRetryRequest, from a real server, completing.
///
/// The client sends a P-384 key share to a server that only speaks X25519, so
/// the server asks it to try again. That exercises the one piece of this
/// module with no analogue anywhere else: RFC 8446 §4.4.1 replaces the first
/// ClientHello in the transcript with a synthetic `message_hash` message
/// wrapping its hash.
///
/// The substitution is invisible until it is wrong. A client that skipped it
/// would negotiate everything correctly, derive keys from a transcript the
/// server does not share, and fail at the server's Finished with an error that
/// looks like corruption. Only a server that actually retries can tell the
/// difference, which is why this test needs one.
#[test]
fn a_hello_retry_request_completes_against_rustls() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    // The key share goes to the first group; X25519 is offered so the server
    // has something to ask for.
    let config = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::SecP384R1, NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };

    let mut server = x25519_only_server(&pki);
    let (mut client, mut to_server) = ClientHandshake::start(&config).expect("start");

    let mut retried = false;
    for _ in 0..8 {
        let mut stream = pump_server(&mut server, &to_server);
        to_server.clear();
        for record in take_records(&mut stream) {
            // A HelloRetryRequest is a ServerHello with the sentinel random,
            // so catch it on the way past to prove the path was taken rather
            // than inferring it from the handshake completing.
            if record[0] == 22 {
                if let Ok(parsed) = messages(&record[5..]) {
                    if parsed[0].typ == HandshakeType::ServerHello {
                        let hello = ServerHello::parse(parsed[0].body).expect("parses");
                        retried |= hello.is_hello_retry_request();
                    }
                }
            }
            to_server.extend_from_slice(&client.read_record(&record).expect("read_record"));
        }
        if client.is_finished() {
            break;
        }
    }

    assert!(retried, "the server never sent a HelloRetryRequest");
    let connection = client
        .into_connection()
        .expect("the handshake completes after a retry");
    assert_eq!(connection.peer_certificates().len(), 2);
}

/// A second HelloRetryRequest is refused. RFC 8446 §4.1.4: one retry, and a
/// client that allowed two could be made to loop forever.
#[test]
fn a_second_hello_retry_request_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let config = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::SecP384R1, NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };

    let mut server = x25519_only_server(&pki);
    let (mut client, to_server) = ClientHandshake::start(&config).expect("start");

    // Take the server's genuine HelloRetryRequest...
    let mut stream = pump_server(&mut server, &to_server);
    let records = take_records(&mut stream);
    let retry = records
        .iter()
        .find(|r| r[0] == 22)
        .expect("a HelloRetryRequest")
        .clone();

    // ...and send it twice.
    client
        .read_record(&retry)
        .expect("the first retry is accepted");
    assert_eq!(
        client.read_record(&retry),
        Err(ClientError::RepeatedHelloRetryRequest),
        "a second HelloRetryRequest was accepted"
    );
}

/// A `key_share` labelled with the wrong group is refused where the
/// disagreement happens, not three messages later.
///
/// This exists because a mutation deleting the check survived, and chasing it
/// showed the check is not doing what it looks like it does: `agree` uses this
/// client's own group whatever the label claims, so a mislabelled share yields
/// a different secret and fails at the server's Finished regardless. The check
/// buys a diagnosable error rather than a mysterious one — which is worth
/// having and worth not overstating, so this test pins the distinction by
/// asserting the *specific* error rather than merely that something failed.
#[test]
fn a_key_share_labelled_with_the_wrong_group_is_refused_as_such() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let error = against_test_server(&pki, Shape::WrongKeyShareGroup)
        .expect_err("a mislabelled key_share was accepted");
    assert_eq!(
        error,
        ClientError::BadKeyShare,
        "refused, but not at the point of the disagreement"
    );
}

// ---------------------------------------------------------------------------
// Post-handshake
// ---------------------------------------------------------------------------

/// A NewSessionTicket must never reach the caller as application data.
///
/// This client does not offer `psk_key_exchange_modes`, so RFC 8446 §4.2.9
/// says a conforming server will never send it a ticket — and `rustls`
/// duly does not, which is why an earlier version of this test passed while
/// exercising nothing. A mutation that returned tickets as application data
/// survived the whole suite.
///
/// The handling is still worth having and worth testing. The cost of getting
/// it wrong is not a failed handshake, it is a caller handed handshake bytes
/// as if the server had sent them — a protocol surprise turned into silent
/// data corruption. So the test server sends one, because a conforming peer
/// will not.
#[test]
fn a_session_ticket_is_never_handed_to_the_caller_as_data() {
    use rusty_tls::handrolled::record::ContentType;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let (mut connection, mut server) =
        established_with_test_server(&pki, Shape::Correct).expect("completes");

    // A NewSessionTicket with a plausible body.
    let mut body = rusty_tls::handrolled::wire::Writer::new();
    body.u32(7200); // ticket_lifetime
    body.u32(0); // ticket_age_add
    body.vector_u8(|w| w.bytes(b"nonce"));
    body.vector_u16(|w| w.bytes(b"an opaque ticket"));
    body.vector_u16(|_| {}); // extensions
    let ticket = Message::encode(HandshakeType::NewSessionTicket, &body.into_vec());
    let record = server.seal(ContentType::Handshake, &ticket);

    let incoming = connection.read(&record).expect("the ticket is tolerated");
    let Incoming::Ticket(session) = incoming else {
        panic!("a session ticket surfaced as {incoming:?} rather than a Ticket");
    };
    assert_eq!(session.ticket, b"an opaque ticket");
    assert_eq!(session.lifetime, 7200);
    // The derived key is the whole point of surfacing it: a Session whose PSK
    // was empty would look resumable and be useless.
    assert!(!session.psk().is_empty(), "the session carries no key");
    assert_eq!(
        session.psk().len(),
        32,
        "a SHA-256 session's PSK should be 32 octets"
    );

    // And the connection still works.
    let data = server.seal(ContentType::ApplicationData, b"after the ticket");
    assert_eq!(
        connection.read(&data).expect("read"),
        Incoming::Application(b"after the ticket".to_vec())
    );
}

/// A KeyUpdate rekeys the receiving direction, and one that asks for a reply
/// gets one.
///
/// RFC 8446 §4.6.3. A client that ignored a KeyUpdate would fail to decrypt
/// everything the server sent afterwards, which looks like a broken
/// connection rather than a missing feature.
#[test]
fn a_key_update_rekeys_the_connection() {
    use rusty_tls::handrolled::record::ContentType;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let (mut connection, mut server) =
        established_with_test_server(&pki, Shape::Correct).expect("completes");

    // Ordinary data first, so the sequence numbers are not both zero.
    let data = server.seal(ContentType::ApplicationData, b"before");
    assert_eq!(
        connection.read(&data).expect("read"),
        Incoming::Application(b"before".to_vec())
    );

    // update_requested(1): the client must rekey and answer.
    let update = Message::encode(HandshakeType::KeyUpdate, &[0x01]);
    let record = server.seal(ContentType::Handshake, &update);
    let reply = connection.read(&record).expect("the update is handled");
    match reply {
        Incoming::Reply(bytes) => assert!(!bytes.is_empty(), "an empty reply"),
        other => panic!("update_requested was not answered: {other:?}"),
    }

    // The server now advances its own send key, as a peer does after sending
    // a KeyUpdate. The client must follow.
    server.rekey();
    let data = server.seal(ContentType::ApplicationData, b"after the rekey");
    assert_eq!(
        connection.read(&data).expect("read after rekey"),
        Incoming::Application(b"after the rekey".to_vec()),
        "the client did not rekey its receiving direction"
    );
}

/// `update_not_requested(0)` rekeys without a reply — answering anyway would
/// be an infinite exchange of KeyUpdates.
#[test]
fn a_key_update_that_asks_for_no_reply_gets_none() {
    use rusty_tls::handrolled::record::ContentType;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let (mut connection, mut server) =
        established_with_test_server(&pki, Shape::Correct).expect("completes");

    let update = Message::encode(HandshakeType::KeyUpdate, &[0x00]);
    let record = server.seal(ContentType::Handshake, &update);
    assert_eq!(
        connection.read(&record).expect("handled"),
        Incoming::Handled,
        "update_not_requested was answered"
    );

    server.rekey();
    let data = server.seal(ContentType::ApplicationData, b"still working");
    assert_eq!(
        connection.read(&data).expect("read"),
        Incoming::Application(b"still working".to_vec())
    );
}

// ---------------------------------------------------------------------------
// Hostile input
// ---------------------------------------------------------------------------

/// xorshift64*, so a failure replays from the seed printed with it.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }
    fn byte(&mut self) -> u8 {
        self.next_u64() as u8
    }
}

/// Nothing a peer can put on the wire makes the client panic.
///
/// The invariant is deliberately just that, rather than "never completes a
/// handshake". A corrupted record stream cannot complete one for a reason that
/// has nothing to do with this code — the records are bound to one client's
/// ephemeral key — so asserting it would look like a security property and be
/// a tautology.
///
/// What is not a tautology is the panic. A client is fed bytes by whoever it
/// dialled, and an index out of range in a handshake parser is a denial of
/// service reachable from the first flight. That is the same bug class as the
/// SAN iterator that stage 2a's fuzzer found.
#[test]
fn no_corrupted_record_stream_makes_the_client_panic() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let mut rng = Rng::new(0x5eed_0100);

    let rounds = std::env::var("RUSTY_TLS_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400usize);

    let mut completed = 0usize;
    let mut reached_flight = 0usize;

    for round in 0..rounds {
        let seen = std::cell::Cell::new(0usize);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handshake_with(&pki, SERVER, |index, mut record| {
                seen.set(index + 1);
                // Three quarters of rounds corrupt exactly one record, so the
                // rest of the stream stays well-formed and the client gets
                // deep into the flight before anything is wrong.
                if rng.below(4) != 0 && !record.is_empty() {
                    let at = rng.below(record.len());
                    record[at] ^= 1 << rng.below(8);
                }
                Some(record)
            })
        }));

        match outcome {
            // A completion is not alarming and not luck: a corrupted
            // change_cipher_spec changes nothing, because that record is
            // discarded by design whatever it contains. What must still hold
            // is that a handshake which completed authenticated the real
            // server — corruption may be harmless, but it must never be
            // *useful* to an attacker.
            Ok(Ok(established)) => {
                completed += 1;
                assert_eq!(
                    established.connection.peer_certificates()[0],
                    pki.leaf_der,
                    "a corrupted stream completed against a different certificate"
                );
            }
            Ok(Err(_)) => {}
            Err(_) => panic!("the client panicked at round {round} (seed 0x5eed0100)"),
        }
        if seen.get() >= 2 {
            reached_flight += 1;
        }
    }

    // A fuzzer that never gets past the first record is testing the header
    // check and nothing else.
    assert!(
        reached_flight * 100 / rounds >= 50,
        "only {reached_flight} of {rounds} rounds reached a second record"
    );
    println!("of {rounds} corrupted streams, {completed} still completed");
}

/// Random bytes framed as records, straight at a fresh client.
#[test]
fn random_records_never_panic() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let mut rng = Rng::new(0x5eed_0101);
    for round in 0..2_000 {
        let (mut client, _) = ClientHandshake::start(&config).expect("start");

        let body: Vec<u8> = (0..rng.below(300)).map(|_| rng.byte()).collect();
        let mut record = vec![
            // Mostly a plausible content type, sometimes anything at all.
            if rng.below(4) == 0 {
                rng.byte()
            } else {
                [20u8, 21, 22, 23][rng.below(4)]
            },
            0x03,
            0x03,
        ];
        record.extend_from_slice(&(body.len() as u16).to_be_bytes());
        record.extend_from_slice(&body);
        // Sometimes lie about the length, which is the malformation a
        // deframer is most likely to mishandle.
        if rng.below(3) == 0 {
            record[3] = rng.byte();
            record[4] = rng.byte();
        }

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.read_record(&record)))
                .unwrap_or_else(|_| panic!("panicked at round {round} on {record:02x?}"));

        assert!(
            !client.is_finished(),
            "a random record completed a handshake"
        );
        let _ = result;
    }
}

/// A flight split across records is reassembled, whatever the split.
///
/// This exists because the coverage it provides was assumed and absent.
/// `rustls` sends its whole flight in one protected record, and so does every
/// server `handrolled_interop` reaches — so `complete_prefix` and the client's
/// reassembly buffer were being carried by tests that never made them work.
///
/// Handshake message boundaries and record boundaries are unrelated by
/// design, so this walks a range of chunk sizes: sizes that split *between*
/// messages, sizes that split *inside* one, and one octet at a time, which
/// puts every message across many records and leaves a partial message in the
/// buffer after almost every read.
#[test]
fn a_flight_split_across_records_is_reassembled() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        cipher_suites: &[CipherSuite::TLS_AES_128_GCM_SHA256],
        identity: None,
        resumption: None,
    };

    for fragment in [1usize, 2, 3, 5, 17, 64, 100, 255, 256, 511, 1024] {
        let (mut client, hello) = ClientHandshake::start(&config).expect("start");
        let server = TestServer {
            pki: &pki,
            shape: Shape::Correct,
            fragment: Some(fragment),
        };
        let (mut stream, _) = server.respond(&hello);

        let records = take_records(&mut stream);
        assert!(
            records.len() > 2 || fragment >= 1024,
            "a fragment size of {fragment} did not actually split the flight"
        );

        for record in records {
            client
                .read_record(&record)
                .unwrap_or_else(|e| panic!("fragment {fragment}: {e}"));
        }
        assert!(
            client.is_finished(),
            "a flight split into {fragment}-octet records did not complete"
        );
    }
}

/// The reassembly buffer must not grow without bound.
///
/// A peer that sends a handshake header claiming a huge length and then
/// dribbles bytes would, in a naive client, be allowed to allocate as much as
/// it liked. This does not assert a specific bound — there is none in the
/// implementation — but it does pin that a stream of maximum-size records
/// carrying no complete message is refused or absorbed rather than accepted
/// as progress.
#[test]
fn a_peer_cannot_drive_the_handshake_with_incomplete_messages() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let (mut client, _) = ClientHandshake::start(&config).expect("start");

    // A ServerHello header claiming three megabytes, then nothing that
    // completes it. Each record is plaintext, which is all that is legal
    // before the ServerHello anyway.
    let mut body = vec![0x02u8, 0x30, 0x00, 0x00]; // ServerHello, 0x300000 long
    body.extend_from_slice(&[0u8; 60]);
    let mut record = vec![22u8, 0x03, 0x03];
    record.extend_from_slice(&(body.len() as u16).to_be_bytes());
    record.extend_from_slice(&body);

    for _ in 0..64 {
        // Never completes a message, so never completes a handshake.
        let _ = client.read_record(&record);
        assert!(
            !client.is_finished(),
            "an incomplete message completed a handshake"
        );
    }
}

// ---------------------------------------------------------------------------
// The version boundary — stage 4
// ---------------------------------------------------------------------------

/// A TLS 1.2-only server, which is the condition stage 4 is gated on and
/// which no reachable network peer provides.
fn tls12_only_server(pki: &Pki) -> rustls::ServerConnection {
    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
        .with_no_client_auth()
        .with_single_cert(pki.chain.clone(), pki.key.clone_key())
        .expect("server config");
    rustls::ServerConnection::new(Arc::new(config)).expect("server connection")
}

/// A server that cannot speak TLS 1.3 is refused, and the refusal says so in
/// the peer's own words.
///
/// This is the stage 4 trigger, manufactured. The issue defines TLS 1.2 as
/// work to do "only if a real peer forces it", and no peer reachable over the
/// network does — every endpoint tried terminates at a TLS 1.3 gateway, so the
/// condition cannot even be observed there. `rustls` restricted to TLS 1.2 can
/// provide it on demand.
///
/// What the client does today is refuse, which is correct. What it now also
/// does is report *why*: the server sends a fatal `protocol_version` alert,
/// and that is the whole difference between "the handshake failed" and "this
/// server is too old for this client". Before this, the alert was discarded as
/// an unexpected content type.
#[test]
fn a_server_that_cannot_speak_tls13_is_refused_in_its_own_words() {
    use rusty_tls::handrolled::client::{AlertDescription, AlertLevel};

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let mut server = tls12_only_server(&pki);
    let (mut client, to_server) = ClientHandshake::start(&config).expect("start");

    let (mut stream, refusal) = pump_server_allowing_refusal(&mut server, &to_server);
    assert!(
        refusal.is_some(),
        "the TLS 1.2-only server accepted a TLS 1.3-only ClientHello"
    );
    let records = take_records(&mut stream);
    assert!(!records.is_empty(), "the server said nothing at all");

    let error = client
        .read_record(&records[0])
        .expect_err("a TLS 1.2-only server completed a handshake");

    match error {
        ClientError::PeerAlert(alert) => {
            assert_eq!(alert.level, AlertLevel::Fatal);
            assert_eq!(
                alert.description,
                AlertDescription::PROTOCOL_VERSION,
                "the alert was not about the version"
            );
            assert!(
                alert.description.to_string().contains("protocol_version"),
                "the alert does not name itself: {}",
                alert.description
            );
        }
        other => panic!("expected a protocol_version alert, got {other:?}"),
    }
}

/// The RFC 8446 §4.1.3 downgrade sentinel distinguishes an old server from an
/// active downgrade.
///
/// Both are refused — this changes which error is returned, not whether the
/// handshake proceeds — and the distinction is worth drawing because the two
/// are the same bytes on the wire and very different problems. A server that
/// sets the sentinel *does* support TLS 1.3, so a ServerHello without
/// `supported_versions` means the ClientHello it saw was not the one that was
/// sent.
#[test]
fn the_downgrade_sentinel_is_told_apart_from_an_old_server() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    // Take a genuine ServerHello and strip `supported_versions`, with and
    // without the sentinel in `random`.
    let mut server = rustls_server(&pki);
    let (_, hello) = ClientHandshake::start(&config).expect("start");
    let mut stream = pump_server(&mut server, &hello);
    let genuine = take_records(&mut stream).remove(0);

    let build = |random: &[u8]| -> Vec<u8> {
        let parsed = messages(&genuine[5..]).expect("parses");
        let body = ServerHello::parse(parsed[0].body).expect("parses");
        let mut rewritten = body.clone();
        rewritten.random = random;
        rewritten
            .extensions
            .retain(|e| e.typ != rusty_tls::handrolled::handshake::extension::SUPPORTED_VERSIONS);
        let encoded = Message::encode(HandshakeType::ServerHello, &rewritten.encode());
        let mut record = vec![22u8, 0x03, 0x03];
        record.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
        record.extend_from_slice(&encoded);
        record
    };

    // An ordinary old server: no sentinel.
    let mut plain = [0x5au8; 32];
    plain[24..].copy_from_slice(&[0x11; 8]);
    let (mut client, _) = ClientHandshake::start(&config).expect("start");
    assert_eq!(
        client.read_record(&build(&plain)),
        Err(ClientError::NotTls13)
    );

    // A TLS 1.3-capable server that negotiated 1.2 anyway.
    let mut sentinel = [0x5au8; 32];
    sentinel[24..].copy_from_slice(b"DOWNGRD\x01");
    let (mut client, _) = ClientHandshake::start(&config).expect("start");
    assert_eq!(
        client.read_record(&build(&sentinel)),
        Err(ClientError::DowngradeDetected),
        "an active downgrade was reported as an old server"
    );

    // And the TLS 1.1-and-below variant.
    let mut older = [0x5au8; 32];
    older[24..].copy_from_slice(b"DOWNGRD\x00");
    let (mut client, _) = ClientHandshake::start(&config).expect("start");
    assert_eq!(
        client.read_record(&build(&older)),
        Err(ClientError::DowngradeDetected)
    );
}

/// An orderly close is not a failure.
///
/// Before alerts were parsed, a `close_notify` surfaced as an unexpected
/// content type, and the interop suite worked around it by treating that as
/// "the correct place to stop" — a missing feature described as correct
/// behaviour.
#[test]
fn a_close_notify_is_reported_as_a_close_not_an_error() {
    use rusty_tls::handrolled::record::ContentType;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let (mut connection, mut server) =
        established_with_test_server(&pki, Shape::Correct).expect("completes");

    let record = server.seal(ContentType::Alert, &[0x01, 0x00]); // warning, close_notify
    assert_eq!(
        connection.read(&record).expect("a close is not an error"),
        Incoming::Closed
    );
}

/// The alert level is read from the wire, not assumed.
///
/// `close_notify` is a warning and a `decrypt_error` is fatal; a client that
/// reported every alert as fatal would be inventing severity it was told.
#[test]
fn an_alerts_level_is_the_one_the_peer_sent() {
    use rusty_tls::handrolled::client::{AlertDescription, AlertLevel};
    use rusty_tls::handrolled::record::ContentType;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let (mut connection, mut server) =
        established_with_test_server(&pki, Shape::Correct).expect("completes");

    // warning(1), user_canceled(90) — a warning that is not a close.
    let record = server.seal(ContentType::Alert, &[0x01, 0x5a]);
    match connection.read(&record) {
        Err(ClientError::PeerAlert(alert)) => {
            assert_eq!(
                alert.level,
                AlertLevel::Warning,
                "a warning was reported as something else"
            );
            assert_eq!(alert.description, AlertDescription(90));
        }
        other => panic!("expected a warning alert, got {other:?}"),
    }

    // An unrecognised level is preserved rather than collapsed.
    let (mut connection, mut server) =
        established_with_test_server(&pki, Shape::Correct).expect("completes");
    let record = server.seal(ContentType::Alert, &[0x07, 0x28]);
    match connection.read(&record) {
        Err(ClientError::PeerAlert(alert)) => {
            assert_eq!(alert.level, AlertLevel::Unknown(7));
        }
        other => panic!("expected an unknown level, got {other:?}"),
    }
}

/// An alert arriving where the encrypted flight should be is reported as the
/// alert it is, not as a surprising content type.
///
/// This is where a real server reports that it disliked something about the
/// ClientHello it could only discover after replying — an unacceptable
/// signature algorithm, say. Losing the description there loses the only
/// explanation anyone will get.
#[test]
fn an_alert_inside_the_flight_is_reported_as_an_alert() {
    use rusty_tls::handrolled::client::AlertDescription;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let error = against_test_server(&pki, Shape::AlertInsteadOfFlight)
        .expect_err("an alert instead of a flight completed a handshake");

    match error {
        ClientError::PeerAlert(alert) => {
            assert_eq!(alert.description, AlertDescription::HANDSHAKE_FAILURE);
        }
        other => panic!("an encrypted alert was not reported as one: {other:?}"),
    }
}

/// Any other alert after the handshake is an error, and names itself.
#[test]
fn a_fatal_alert_after_the_handshake_is_an_error() {
    use rusty_tls::handrolled::client::AlertDescription;
    use rusty_tls::handrolled::record::ContentType;

    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let (mut connection, mut server) =
        established_with_test_server(&pki, Shape::Correct).expect("completes");

    let record = server.seal(ContentType::Alert, &[0x02, 0x33]); // fatal, decrypt_error
    match connection.read(&record) {
        Err(ClientError::PeerAlert(alert)) => {
            assert_eq!(alert.description, AlertDescription::DECRYPT_ERROR);
        }
        other => panic!("a fatal alert was not reported as one: {other:?}"),
    }
}

/// An alert body that is not two octets is malformed, and is not guessed at.
#[test]
fn a_malformed_alert_is_refused_rather_than_interpreted() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    for body in [vec![], vec![0x02], vec![0x02, 0x46, 0x00]] {
        let (mut client, _) = ClientHandshake::start(&config).expect("start");
        let mut record = vec![21u8, 0x03, 0x03];
        record.extend_from_slice(&(body.len() as u16).to_be_bytes());
        record.extend_from_slice(&body);
        assert!(
            matches!(
                client.read_record(&record),
                Err(ClientError::UnexpectedContentType(_))
            ),
            "a {}-octet alert body was interpreted",
            body.len()
        );
    }
}

/// The ServerHello must echo the `legacy_session_id` that was sent.
///
/// RFC 8446 §4.1.3. It is a cheap binding between the ClientHello that went
/// out and the ServerHello that came back, and this test exists because a
/// mutation on the *server* side — making it stop echoing — was accepted by
/// this client. Mutating one half found a gap in the other.
#[test]
fn a_server_hello_that_does_not_echo_the_session_id_is_refused() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let mut server = rustls_server(&pki);
    let (mut client, hello) = ClientHandshake::start(&config).expect("start");
    let mut stream = pump_server(&mut server, &hello);
    let genuine = take_records(&mut stream).remove(0);

    let parsed = messages(&genuine[5..]).expect("parses");
    let body = ServerHello::parse(parsed[0].body).expect("parses");

    for wrong in [&[][..], &[0x99u8; 32][..]] {
        let mut rewritten = body.clone();
        rewritten.session_id = wrong;
        let encoded = Message::encode(HandshakeType::ServerHello, &rewritten.encode());
        let mut record = vec![22u8, 0x03, 0x03];
        record.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
        record.extend_from_slice(&encoded);

        let (mut fresh, _) = ClientHandshake::start(&config).expect("start");
        assert_eq!(
            fresh.read_record(&record),
            Err(ClientError::SessionIdMismatch),
            "a {}-octet session id echo was accepted",
            wrong.len()
        );
    }

    // And the genuine one is still accepted, so the check is not just refusing
    // everything.
    assert!(client.read_record(&genuine).is_ok());
}

/// A retried ClientHello keeps the `random` and `legacy_session_id` of the
/// first.
///
/// RFC 8446 §4.1.2 enumerates what a second ClientHello may change, and
/// neither is on the list. This was wrong until the session-id echo above went
/// in: the retry path built a fresh hello with fresh values, which `rustls`
/// happens to accept and no server is obliged to.
#[test]
fn a_retried_client_hello_keeps_its_identity() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let config = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: &[NamedGroup::SecP384R1, NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };

    let mut server = x25519_only_server(&pki);
    let (mut client, first) = ClientHandshake::start(&config).expect("start");

    let parsed = messages(&first[5..]).expect("parses");
    let before = ClientHello::parse(parsed[0].body).expect("parses");
    let (random, session_id) = (before.random.to_vec(), before.session_id.to_vec());

    let mut stream = pump_server(&mut server, &first);
    let retry = take_records(&mut stream)
        .into_iter()
        .find(|r| r[0] == 22)
        .expect("a HelloRetryRequest");

    let second = client.read_record(&retry).expect("the retry is accepted");
    assert!(!second.is_empty(), "no second ClientHello was produced");

    let parsed = messages(&second[5..]).expect("parses");
    let after = ClientHello::parse(parsed[0].body).expect("parses");

    assert_eq!(after.random, &random[..], "the retry changed `random`");
    assert_eq!(
        after.session_id,
        &session_id[..],
        "the retry changed `legacy_session_id`"
    );
    // The key share is the one thing that must change.
    use rusty_tls::handrolled::handshake::{extension, find};
    assert_ne!(
        find(&before.extensions, extension::KEY_SHARE),
        find(&after.extensions, extension::KEY_SHARE),
        "the retry reused the key share the server rejected"
    );
}

// ---------------------------------------------------------------------------
// Client certificates — rusty_tls#42
// ---------------------------------------------------------------------------

/// A `rustls` server that requires the client to authenticate.
///
/// This is the oracle that matters for client certificates. `rustls` verifies
/// the chain *and* the CertificateVerify signature — including that it was
/// made with the client context string over the right transcript — so a
/// completed handshake here is evidence about all three. A test server of this
/// crate's own making would agree with whatever this crate did.
fn rustls_server_requiring_client_auth(pki: &Pki, client_root: &[u8]) -> rustls::ServerConnection {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(client_root.to_vec()))
        .expect("the client root is acceptable to rustls");

    let verifier = rustls::server::WebPkiClientVerifier::builder(std::sync::Arc::new(roots))
        .build()
        .expect("client verifier");

    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_client_cert_verifier(verifier)
        .with_single_cert(pki.chain.clone(), pki.key.clone_key())
        .expect("server config");
    rustls::ServerConnection::new(Arc::new(config)).expect("server connection")
}

/// Drive this client against a `rustls` server that demands a certificate.
///
/// `identity` is what the client will present; `None` means it has nothing,
/// which is a case with its own correct answer rather than an error.
fn against_client_auth(
    pki: &Pki,
    client_pki: &Pki,
    identity: Option<&ClientIdentity<'_>>,
) -> Result<(), ClientError> {
    let root = anchor(&pki.root_der);
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
        identity,
        resumption: None,
    };

    let mut server = rustls_server_requiring_client_auth(pki, &client_pki.root_der);
    let (mut client, mut to_server) = ClientHandshake::start(&config)?;

    for _ in 0..8 {
        let mut from_server = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&to_server);
            while server.read_tls(&mut cursor).expect("read_tls") > 0 {
                // `process_new_packets` is where rustls checks the client's
                // certificate and signature, so its error is the verdict this
                // test is after.
                server
                    .process_new_packets()
                    .map_err(|_| ClientError::Failed)?;
            }
            server
                .process_new_packets()
                .map_err(|_| ClientError::Failed)?;
            while server.wants_write() {
                server.write_tls(&mut from_server).expect("write_tls");
            }
        }
        to_server.clear();

        let mut stream = from_server;
        for record in take_records(&mut stream) {
            to_server.extend_from_slice(&client.read_record(&record)?);
        }

        if client.is_finished() {
            // Deliver the client's last flight so rustls actually checks it —
            // without this the test would pass on a signature nobody read.
            let mut cursor = std::io::Cursor::new(&to_server);
            while server.read_tls(&mut cursor).expect("read_tls") > 0 {
                server
                    .process_new_packets()
                    .map_err(|_| ClientError::Failed)?;
            }
            server
                .process_new_packets()
                .map_err(|_| ClientError::Failed)?;
            assert!(!server.is_handshaking(), "rustls never finished");
            return Ok(());
        }
        if to_server.is_empty() {
            break;
        }
    }
    Err(ClientError::Failed)
}

/// The headline: this client authenticates itself to a `rustls` server.
///
/// `rustls` checks the chain against a root it was given, and checks the
/// CertificateVerify — context string, transcript, and all. Nothing in this
/// crate is asked whether its own signature is correct.
#[test]
fn this_client_authenticates_itself_to_a_rustls_server() {
    let server_pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let client_pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, "client.example");
    let key = SigningKey::ecdsa_p256(&client_pki.leaf_pkcs8).expect("client key");
    let identity = ClientIdentity {
        certificates: &client_pki.chain_der,
        key: &key,
    };

    against_client_auth(&server_pki, &client_pki, Some(&identity))
        .expect("rustls rejected this client's certificate");
}

/// A client with no identity answers with an empty Certificate, and the server
/// gets to decide.
///
/// The server here requires a certificate, so it refuses — which is the point.
/// The client's job is to say "I have nothing" in the conforming way rather
/// than to abort the handshake on the server's behalf, and this shows both
/// halves of that: the empty message is well-formed enough for `rustls` to
/// read, and `rustls` is the one that says no.
#[test]
fn a_client_with_no_identity_sends_an_empty_certificate_and_is_refused_by_the_server() {
    let server_pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let client_pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, "client.example");

    let result = against_client_auth(&server_pki, &client_pki, None);
    assert!(
        result.is_err(),
        "a server requiring a certificate accepted a client with none"
    );
}

// ---------------------------------------------------------------------------
// Session tickets — rusty_tls#43, stage one
// ---------------------------------------------------------------------------

/// A real `rustls` server now sends this client a NewSessionTicket.
///
/// This is the measurement ADR-0003 was written about. Until the client
/// offered `psk_key_exchange_modes`, RFC 8446 §4.2.9 meant a conforming server
/// sent no ticket **ever** — so the client's ticket-handling branch could not
/// be reached, and the test that claimed to cover it was green and vacuous. A
/// mutation returning a ticket as application data survived it.
///
/// The assertion is deliberately "at least one arrived", not an exact count:
/// how many tickets a server chooses to issue is that server's business, and
/// pinning `rustls`' current answer would make this fail when `rustls` changes
/// something it is entitled to change. What matters is that the number is no
/// longer zero.
#[test]
fn a_rustls_server_now_sends_session_tickets() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
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
        identity: None,
        resumption: None,
    };

    let mut server = rustls_server(&pki);
    let (mut client, mut to_server) = ClientHandshake::start(&config).expect("start");

    let mut connection = None;
    for _ in 0..8 {
        let mut stream = pump_server(&mut server, &to_server);
        to_server.clear();
        for record in take_records(&mut stream) {
            to_server.extend_from_slice(&client.read_record(&record).expect("client"));
        }
        if client.is_finished() {
            connection = Some(client.into_connection().expect("connection"));
            break;
        }
    }
    let mut connection = connection.expect("the handshake did not complete");

    // Deliver the client's Finished; rustls issues its tickets in response.
    let mut stream = pump_server(&mut server, &to_server);
    let mut tickets = 0usize;
    for record in take_records(&mut stream) {
        match connection.read(&record).expect("a post-handshake record") {
            Incoming::Ticket(session) => {
                assert!(
                    !session.psk().is_empty(),
                    "a ticket arrived with no derived key"
                );
                tickets += 1;
            }
            Incoming::Application(data) => {
                panic!("a post-handshake message surfaced as data: {data:02x?}")
            }
            _ => {}
        }
    }

    assert!(
        tickets > 0,
        "rustls still sent no NewSessionTicket — is psk_key_exchange_modes being offered?"
    );
    println!("rustls sent {tickets} session ticket(s)");
}

// ---------------------------------------------------------------------------
// Resumption — rusty_tls#43, the acceptance criterion
// ---------------------------------------------------------------------------

/// A `rustls` server config that can resume, shared across two connections.
///
/// The sharing is the point. `rustls` keeps its stateful ticket store on the
/// `ServerConfig`, so two `ServerConnection`s built from the same `Arc` are the
/// same server as far as resumption is concerned, and two built from different
/// ones are not — which is exactly the distinction a resumption test needs to
/// make.
fn resumable_server_config(pki: &Pki) -> Arc<rustls::ServerConfig> {
    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(pki.chain.clone(), pki.key.clone_key())
        .expect("server config");
    Arc::new(config)
}

/// What one handshake against a shared server produced.
#[derive(Debug)]
struct Ran {
    connection: rusty_tls::handrolled::client::Connection,
    sessions: Vec<Session>,
}

/// Drive one handshake to completion against a `rustls` server built from
/// `server_config`, collecting every ticket that follows.
///
/// `tamper` sees the ClientHello record on its way out. It is the only hook of
/// its kind here because the binder is in that record and nowhere else, so it
/// is the only place a corruption tests what these tests are about.
///
/// A `rustls` refusal comes back as `Err` rather than a panic: for the binder
/// test, the refusal *is* the expected result, and asserting it away would
/// leave nothing checking that `rustls` verifies binders at all.
fn run_against(
    config: &ClientConfig<'_>,
    server_config: &Arc<rustls::ServerConfig>,
    tamper: impl FnOnce(Vec<u8>) -> Vec<u8>,
) -> Result<Ran, String> {
    let mut server =
        rustls::ServerConnection::new(server_config.clone()).expect("server connection");
    let (client, hello) = ClientHandshake::start(config).map_err(|err| format!("start: {err}"))?;

    let mut handshake = Some(client);
    let mut connection: Option<rusty_tls::handrolled::client::Connection> = None;
    let mut sessions = Vec::new();
    let mut to_server = tamper(hello);

    for _ in 0..8 {
        let had_input = !to_server.is_empty();
        let (mut stream, refusal) = pump_server_allowing_refusal(&mut server, &to_server);
        to_server.clear();
        if let Some(err) = refusal {
            return Err(err);
        }

        let records = take_records(&mut stream);
        if records.is_empty() && !had_input {
            break;
        }
        for record in records {
            if let Some(client) = handshake.as_mut() {
                let reply = client
                    .read_record(&record)
                    .map_err(|err| format!("client: {err}"))?;
                to_server.extend_from_slice(&reply);
                if client.is_finished() {
                    connection = Some(
                        handshake
                            .take()
                            .expect("just borrowed")
                            .into_connection()
                            .map_err(|err| format!("into_connection: {err}"))?,
                    );
                }
            } else if let Some(connection) = connection.as_mut() {
                match connection
                    .read(&record)
                    .map_err(|err| format!("post-handshake: {err}"))?
                {
                    Incoming::Ticket(session) => sessions.push(*session),
                    Incoming::Application(data) => {
                        return Err(format!(
                            "a post-handshake message surfaced as data: {data:02x?}"
                        ))
                    }
                    _ => {}
                }
            }
        }

        if connection.is_some() && to_server.is_empty() {
            break;
        }
    }

    match connection {
        Some(connection) => Ok(Ran {
            connection,
            sessions,
        }),
        None => Err("the handshake did not complete".to_string()),
    }
}

fn resumption_config<'a>(
    anchors: &'a [TrustAnchor<'a>],
    resumption: Option<Resumption<'a>>,
) -> ClientConfig<'a> {
    ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors,
        path: options(),
        groups: &[NamedGroup::X25519],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption,
    }
}

/// **The test this issue exists for.** A second connection resumes the first.
///
/// Everything `rusty_tls#43` merged before this — the PSK derived from a
/// ticket in `0.6.0`, `binder_key` and `psk_binder` in `0.7.0` — was tested for
/// *shape* and not for *value*. The issue's own status comment measured it:
/// swapping `"res binder"` for `"ext binder"` passed all five binder tests. So
/// did the `res master` transcript point and the `"resumption"` expansion, with
/// nothing checking either.
///
/// This is what checks them, and it checks all of them at once, because a
/// `rustls` server that accepts the binder has independently computed:
///
/// - the same PSK from its own resumption master secret and the ticket's nonce
///   — so `"resumption"` and the `res master` transcript point are right;
/// - the same binder key from that PSK under `"res binder"` — so that label is
///   right;
/// - the same binder over the same truncated ClientHello — so the two-phase
///   encoding truncates in the right place.
///
/// None of those are separately assertable from inside this crate. They are
/// separately *wrong*-able, and this is the one measurement that sees any of
/// them. If it fails, the fault is in one of the four and the error will say
/// `IncorrectBinder`, which does not narrow it — that is the cost of an oracle
/// that only answers yes or no.
#[test]
fn a_second_connection_resumes_the_first_against_rustls() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let server_config = resumable_server_config(&pki);

    let first = run_against(
        &resumption_config(&anchors, None),
        &server_config,
        |hello| hello,
    )
    .expect("the first handshake");
    assert!(
        !first.connection.resumed(),
        "the first connection cannot have resumed anything"
    );
    let first_certificates = first.connection.peer_certificates().to_vec();
    let session = first
        .sessions
        .into_iter()
        .next()
        .expect("rustls issued no ticket to resume with");

    let second = run_against(
        &resumption_config(
            &anchors,
            Some(Resumption {
                session: &session,
                age_ms: 1_000,
            }),
        ),
        &server_config,
        |hello| hello,
    )
    .expect("the second handshake");

    assert!(
        second.connection.resumed(),
        "rustls accepted the handshake but did not accept the pre-shared key"
    );
    // The chain from the connection the ticket came from, carried forward — a
    // resumed handshake carries no Certificate message, and an application that
    // gates on the peer's certificates must not silently see none.
    assert_eq!(
        second.connection.peer_certificates(),
        pki.chain_der.as_slice(),
        "a resumed connection reported the wrong peer"
    );
    assert_eq!(
        first_certificates,
        second.connection.peer_certificates(),
        "the resumed connection's peer differs from the one the ticket came from"
    );
}

/// The binder is actually checked by the server, so the test above means
/// something.
///
/// A positive interop result proves nothing on its own unless the peer would
/// have refused a wrong answer. This corrupts the last octet of the
/// ClientHello — which, because `pre_shared_key` is the last extension and the
/// binders are its last field, is the last octet of the binder — and requires
/// `rustls` to refuse.
///
/// Without this, `a_second_connection_resumes_the_first_against_rustls` would
/// still pass if `rustls` ignored binders entirely, and the whole chain of
/// derivations it claims to verify would be unverified.
#[test]
fn rustls_refuses_a_corrupted_binder() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let server_config = resumable_server_config(&pki);

    let first = run_against(
        &resumption_config(&anchors, None),
        &server_config,
        |hello| hello,
    )
    .expect("the first handshake");
    let session = first
        .sessions
        .into_iter()
        .next()
        .expect("rustls issued no ticket to resume with");

    let refusal = run_against(
        &resumption_config(
            &anchors,
            Some(Resumption {
                session: &session,
                age_ms: 1_000,
            }),
        ),
        &server_config,
        |mut hello| {
            let last = hello.len() - 1;
            hello[last] ^= 0x01;
            hello
        },
    )
    .expect_err("rustls accepted a corrupted binder");

    assert!(
        refusal.to_lowercase().contains("binder"),
        "rustls refused, but not for the binder: {refusal}"
    );
}

/// The `obfuscated_ticket_age` the offer carries is `age_ms + age_add`,
/// modulo 2³².
///
/// **This test pins arithmetic; it does not verify it against a peer.** The
/// distinction matters and is the same one the issue's status comment drew
/// about the binder derivations. A server uses the obfuscated age for
/// anti-replay in 0-RTT and for nothing else — `rustls` accepts a 1-RTT
/// resumption whatever the age says, which is why the mutation replacing
/// `age_add` with zero survived the resumption tests above. No oracle in this
/// repo can catch that, so this checks the formula directly and says out loud
/// that a green tick here is a regression guard rather than an interop result.
#[test]
fn the_offer_carries_the_obfuscated_ticket_age() {
    let pki = pki(&rcgen::PKCS_ECDSA_P256_SHA256, SERVER);
    let root = anchor(&pki.root_der);
    let anchors = [TrustAnchor {
        subject: root.subject(),
        public_key: root.subject_public_key_info(),
        name_constraints: None,
    }];
    let server_config = resumable_server_config(&pki);

    let first = run_against(
        &resumption_config(&anchors, None),
        &server_config,
        |hello| hello,
    )
    .expect("the first handshake");
    let session = first
        .sessions
        .into_iter()
        .next()
        .expect("rustls issued no ticket to resume with");

    // A value chosen so the sum wraps for most `age_add`s a server picks, since
    // wrapping is the part of the formula an implementation gets wrong.
    let age_ms = 4_000_000_000u32;
    let config = resumption_config(
        &anchors,
        Some(Resumption {
            session: &session,
            age_ms,
        }),
    );
    let (_client, hello) = ClientHandshake::start(&config).expect("start");

    let messages = messages(&hello[5..]).expect("the hello parses");
    let hello = ClientHello::parse(messages[0].body).expect("a ClientHello");
    let offer = hello.extensions.last().expect("the hello has extensions");
    assert_eq!(
        offer.typ, 41,
        "pre_shared_key is not the last extension, so the binder covers the wrong bytes"
    );
    let offer = PresharedKeyOffer::parse(offer.data).expect("the offer parses");

    assert_eq!(offer.identities.len(), 1);
    assert_eq!(offer.identities[0].identity, session.ticket.as_slice());
    assert_eq!(
        offer.identities[0].obfuscated_ticket_age,
        age_ms.wrapping_add(session.age_add),
        "the ticket age is not obfuscated with the server's age_add"
    );
    assert_eq!(offer.binders.len(), 1);
    let (_, hash) = session.suite.parts().expect("a known suite");
    assert_eq!(
        offer.binders[0].len(),
        hash.len(),
        "a binder is one hash length, under the PSK's own hash"
    );
}
