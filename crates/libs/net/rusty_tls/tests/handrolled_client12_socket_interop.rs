//! The hand-rolled TLS 1.2 **client**, over a real socket, against `openssl
//! s_server` — stage 4b-iii.
//!
//! `handrolled_client12.rs` drives rustls in memory with whole records handed
//! across as byte slices. Two things that leaves out, and why this file exists:
//!
//! 1. **An unrelated stack.** rustls agreeing with this client is weaker
//!    evidence than OpenSSL agreeing: they are different code, written by
//!    different people from separate readings of RFC 5246 and 7627.
//! 2. **A socket.** A socket delivers a header and a body that may arrive in
//!    separate reads, and the caller has to reassemble before the client sees a
//!    record. The in-memory tests cannot exercise that.
//!
//! It also reaches what rustls cannot be made to do: sign a `ServerKeyExchange`
//! with RSA PKCS#1 v1.5 (`-sigalgs RSA+SHA256`), the TLS 1.2 case that
//! [`verify_tls12_signature`](rusty_tls::handrolled::verify::verify_tls12_signature)
//! exists for.
//!
//! # Why these are `#[ignore]`d, and why CI runs them anyway
//!
//! They are hermetic (loopback and a child process, no network) but depend on
//! the `openssl` binary. `#[ignore]` rather than detect-and-skip, for the
//! reason `handrolled_socket_interop.rs` gives at length: a test that quietly
//! passes when its precondition is absent reports `ok` for a run that did
//! nothing. CI runs them with `-- --ignored` after printing `openssl version`.
//!
//! ```text
//! cargo test --features handrolled-engine --test handrolled_client12_socket_interop \
//!     -- --ignored --nocapture
//! ```

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::PrivatePkcs8KeyDer;
use time::OffsetDateTime;

use rusty_tls::handrolled::client::ClientError;
use rusty_tls::handrolled::client12::{
    CipherSuite12, ClientConfig12, ClientHandshake12, Incoming12,
};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::x509::Certificate;

const SERVER: &str = "tls12.example";
const REQUEST: &[u8] = b"GET / HTTP/1.0\r\n\r\n";

fn options() -> PathOptions {
    PathOptions {
        time: 1_800_000_000,
        max_path_length: 8,
        max_signature_checks: 64,
        required_eku: None,
    }
}

fn dated(params: &mut CertificateParams) {
    params.not_before = OffsetDateTime::from_unix_timestamp(1_577_836_800).expect("not_before");
    params.not_after = OffsetDateTime::from_unix_timestamp(1_893_456_000).expect("not_after");
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.trim().len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s.trim()[i..i + 2], 16).expect("valid hex"))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaf {
    P256,
    P384,
    Rsa,
}

struct Pki {
    root_der: Vec<u8>,
    root_pem: String,
    leaf_pem: String,
    key_pem: String,
    /// The leaf then the root, DER, and the leaf's key as PKCS#8: what a client
    /// presenting this certificate needs.
    chain: Vec<Vec<u8>>,
    pkcs8: Vec<u8>,
}

fn pki(leaf: Leaf, name: &str) -> Pki {
    let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    root_params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("tls12 socket test root".to_string()),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");

    let leaf_key = match leaf {
        Leaf::P256 => KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256),
        Leaf::P384 => KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384),
        Leaf::Rsa => KeyPair::from_pkcs8_der_and_sign_algo(
            // A throwaway 2048-bit RSA key generated for these tests (ring cannot
            // generate RSA keys). It protects nothing.
            &PrivatePkcs8KeyDer::from(unhex(include_str!("data/rsa2048_pkcs8.hex"))),
            &rcgen::PKCS_RSA_SHA256,
        ),
    }
    .expect("leaf key");
    let mut leaf_params = CertificateParams::new(vec![name.to_string()]).expect("params");
    dated(&mut leaf_params);
    let leaf_cert = leaf_params
        .signed_by(&leaf_key, &root, &root_key)
        .expect("leaf");

    Pki {
        root_der: root.der().to_vec(),
        root_pem: root.pem(),
        leaf_pem: leaf_cert.pem(),
        key_pem: leaf_key.serialize_pem(),
        chain: vec![leaf_cert.der().to_vec(), root.der().to_vec()],
        pkcs8: leaf_key.serialize_der(),
    }
}

/// Files for `s_server`, removed on drop.
struct Files(PathBuf);

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl Files {
    fn new(pki: &Pki) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rusty_tls_client12_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("leaf.pem"), &pki.leaf_pem).expect("leaf");
        std::fs::write(dir.join("key.pem"), &pki.key_pem).expect("key");
        std::fs::write(dir.join("root.pem"), &pki.root_pem).expect("root");
        Self(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An `openssl s_server`, killed on drop.
struct Server {
    child: Child,
    port: u16,
    _files: Files,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// Start `openssl s_server -www` with extra arguments, and wait until it
/// accepts connections.
fn serve(pki: &Pki, extra: &[&str]) -> Server {
    let files = Files::new(pki);
    let port = free_port();
    let child = Command::new("openssl")
        .args(["s_server", "-accept", &port.to_string()])
        .args(["-cert", &files.path("leaf.pem")])
        .args(["-cert_chain", &files.path("root.pem")])
        .args(["-key", &files.path("key.pem")])
        .args(["-tls1_2", "-no_ticket", "-www", "-quiet"])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            std::fs::File::create(files.0.join("stderr.txt")).expect("stderr file"),
        ))
        .spawn()
        .expect("openssl s_server starts");

    let mut server = Server {
        child,
        port,
        _files: files,
    };
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let why = std::fs::read_to_string(server._files.0.join("stderr.txt")).unwrap_or_default();
    let status = server.child.try_wait().ok().flatten();
    panic!("openssl s_server did not start on port {port} (exit {status:?}): {extra:?}\n{why}");
}

fn read_record(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    let mut record = header.to_vec();
    record.resize(5 + length, 0);
    stream.read_exact(&mut record[5..])?;
    Ok(record)
}

/// How a conversation can end other than success.
#[derive(Debug)]
enum Talk {
    Refused(ClientError),
    /// A transport failure. Carried so a test failure shows what happened.
    #[allow(dead_code)]
    Io(std::io::Error),
}

impl From<ClientError> for Talk {
    fn from(err: ClientError) -> Self {
        Self::Refused(err)
    }
}
impl From<std::io::Error> for Talk {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// What a successful conversation produced.
struct Reply {
    response: String,
    suite: CipherSuite12,
}

/// Handshake with the server, send a request, read the response to the close.
fn talk(port: u16, config: &ClientConfig12<'_>) -> Result<Reply, Talk> {
    let mut socket = TcpStream::connect(("127.0.0.1", port))?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;

    let (mut handshake, hello) = ClientHandshake12::start(config)?;
    socket.write_all(&hello)?;
    while !handshake.is_finished() {
        let record = read_record(&mut socket)?;
        let reply = handshake.read_record(&record)?;
        if !reply.is_empty() {
            socket.write_all(&reply)?;
        }
    }
    let mut connection = handshake.into_connection()?;
    socket.write_all(&connection.write(REQUEST)?)?;

    let mut body = Vec::new();
    // Until the server closes the socket.
    while let Ok(record) = read_record(&mut socket) {
        match connection.read(&record)? {
            Incoming12::Application(data) => body.extend_from_slice(&data),
            Incoming12::Closed => break,
            Incoming12::Handled => {}
            Incoming12::Reply(bytes) => socket.write_all(&bytes)?,
            _ => {}
        }
    }
    Ok(Reply {
        response: String::from_utf8_lossy(&body).into_owned(),
        suite: connection.suite(),
    })
}

fn anchors(pki: &Pki) -> [TrustAnchor<'_>; 1] {
    let root = Certificate::parse(&pki.root_der).expect("root parses");
    [TrustAnchor::from_certificate(&root)]
}

fn run(
    pki: &Pki,
    extra: &[&str],
    suites: &[CipherSuite12],
    groups: &[NamedGroup],
) -> Result<Reply, Talk> {
    let server = serve(pki, extra);
    let anchors = anchors(pki);
    let config = ClientConfig12 {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups,
        cipher_suites: suites,
        identity: None,
    };
    talk(server.port, &config)
}

const ALL_GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_full_handshake_with_openssl_completes_and_returns_a_response() {
    let reply = run(
        &pki(Leaf::P256, SERVER),
        &[],
        CipherSuite12::SUPPORTED,
        ALL_GROUPS,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        reply.response.starts_with("HTTP/1.0 200 ok"),
        "{}",
        reply.response
    );
    assert!(reply.response.contains("TLSv1.2"), "{}", reply.response);
}

/// All six suites, each against the key type it authenticates with, and OpenSSL
/// reports the one that was negotiated.
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn every_suite_completes_with_openssl_and_is_the_one_negotiated() {
    let cases: [(CipherSuite12, &str, Leaf); 6] = [
        (
            CipherSuite12::ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            "ECDHE-ECDSA-AES128-GCM-SHA256",
            Leaf::P256,
        ),
        (
            CipherSuite12::ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            "ECDHE-RSA-AES128-GCM-SHA256",
            Leaf::Rsa,
        ),
        (
            CipherSuite12::ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            "ECDHE-ECDSA-AES256-GCM-SHA384",
            Leaf::P384,
        ),
        (
            CipherSuite12::ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            "ECDHE-RSA-AES256-GCM-SHA384",
            Leaf::Rsa,
        ),
        (
            CipherSuite12::ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
            "ECDHE-ECDSA-CHACHA20-POLY1305",
            Leaf::P256,
        ),
        (
            CipherSuite12::ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
            "ECDHE-RSA-CHACHA20-POLY1305",
            Leaf::Rsa,
        ),
    ];
    for (suite, openssl_name, leaf) in cases {
        let reply = run(
            &pki(leaf, SERVER),
            &["-cipher", openssl_name],
            &[suite],
            ALL_GROUPS,
        )
        .unwrap_or_else(|e| panic!("{suite:?}: {e:?}"));
        assert_eq!(reply.suite, suite);
        assert!(
            reply
                .response
                .contains(&format!("Cipher is {openssl_name}")),
            "{suite:?}: OpenSSL did not report {openssl_name}: {}",
            reply.response
        );
    }
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn every_curve_completes_with_openssl() {
    for (group, name) in [
        (NamedGroup::X25519, "X25519"),
        (NamedGroup::SecP256R1, "P-256"),
        (NamedGroup::SecP384R1, "P-384"),
    ] {
        // RFC 8422 §5.1: for an ECDSA certificate the client's supported_groups
        // also names the certificate's curve, and OpenSSL enforces it. So the
        // P-256 certificate's curve is offered alongside the one under test; the
        // server's `-groups` restricts which is used for the key exchange.
        let offered = [group, NamedGroup::SecP256R1];
        let reply = run(
            &pki(Leaf::P256, SERVER),
            &["-groups", name],
            CipherSuite12::SUPPORTED,
            &offered,
        )
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(reply.response.contains("TLSv1.2"), "{name}");
    }
}

/// The reason this file exists: OpenSSL can be told to sign the
/// `ServerKeyExchange` with RSA PKCS#1 v1.5, which rustls will not do when PSS
/// is on offer. Each scheme the client offers for RSA is exercised.
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn every_rsa_signature_scheme_is_verified_against_openssl() {
    for sigalgs in [
        "RSA+SHA256",
        "RSA+SHA384",
        "RSA+SHA512",
        "RSA-PSS+SHA256",
        "RSA-PSS+SHA384",
        "RSA-PSS+SHA512",
    ] {
        run(
            &pki(Leaf::Rsa, SERVER),
            &[
                "-cipher",
                "ECDHE-RSA-AES128-GCM-SHA256",
                "-sigalgs",
                sigalgs,
            ],
            CipherSuite12::SUPPORTED,
            ALL_GROUPS,
        )
        .unwrap_or_else(|e| panic!("{sigalgs}: {e:?}"));
    }
}

/// A self-signed Ed25519 certificate made by OpenSSL, used as its own trust
/// anchor. rcgen's Ed25519 keys serialise as PKCS#8 v2 (with the public key),
/// which OpenSSL 3.0's key reader refuses, so this one pair is made by OpenSSL.
/// Returns the pair and a validation time inside the certificate's window.
fn ed25519_via_openssl() -> (Pki, i64) {
    let dir = std::env::temp_dir().join(format!(
        "rusty_tls_ed25519_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let key = dir.join("key.pem");
    let cert = dir.join("cert.pem");
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let status = Command::new("openssl")
        .args([
            "req", "-x509", "-newkey", "ed25519", "-nodes", "-days", "30",
        ])
        .args(["-subj", &format!("/CN={SERVER}")])
        .args(["-addext", &format!("subjectAltName=DNS:{SERVER}")])
        .args(["-keyout", &path(&key), "-out", &path(&cert)])
        .stderr(Stdio::null())
        .status()
        .expect("openssl req");
    assert!(
        status.success(),
        "openssl could not make an Ed25519 certificate"
    );

    let der = Command::new("openssl")
        .args(["x509", "-in", &path(&cert), "-outform", "DER"])
        .output()
        .expect("openssl x509")
        .stdout;
    let pem = std::fs::read_to_string(&cert).expect("cert");
    let key_pem = std::fs::read_to_string(&key).expect("key");
    let _ = std::fs::remove_dir_all(&dir);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    (
        Pki {
            root_der: der,
            root_pem: pem.clone(),
            leaf_pem: pem,
            key_pem,
            chain: Vec::new(),
            pkcs8: Vec::new(),
        },
        now,
    )
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn ed25519_is_verified_against_openssl() {
    let (pki, now) = ed25519_via_openssl();
    let server = serve(
        &pki,
        &[
            "-cipher",
            "ECDHE-ECDSA-AES128-GCM-SHA256",
            "-sigalgs",
            "ed25519",
        ],
    );
    let anchors = anchors(&pki);
    let config = ClientConfig12 {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: PathOptions {
            time: now,
            ..options()
        },
        groups: ALL_GROUPS,
        cipher_suites: CipherSuite12::SUPPORTED,
        identity: None,
    };
    let reply = talk(server.port, &config).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(reply.response.contains("TLSv1.2"), "{}", reply.response);
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn ecdsa_signature_schemes_are_verified_against_openssl() {
    for (leaf, sigalgs) in [
        (Leaf::P256, "ECDSA+SHA256"),
        (Leaf::P384, "ECDSA+SHA384"),
        // TLS 1.2's ECDSA schemes name only the hash, so a key may be signed
        // with the other curve's scheme. OpenSSL does exactly this.
        (Leaf::P384, "ECDSA+SHA256"),
        (Leaf::P256, "ECDSA+SHA384"),
    ] {
        run(
            &pki(leaf, SERVER),
            &[
                "-cipher",
                "ECDHE-ECDSA-AES128-GCM-SHA256",
                "-sigalgs",
                sigalgs,
            ],
            CipherSuite12::SUPPORTED,
            ALL_GROUPS,
        )
        .unwrap_or_else(|e| panic!("{leaf:?} {sigalgs}: {e:?}"));
    }
}

/// OpenSSL asks for a client certificate and does not insist: this client says
/// it has none, and the handshake completes. Where it insists, it refuses.
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_certificate_request_is_answered_with_none() {
    let pki = pki(Leaf::P256, SERVER);
    run(
        &pki,
        &["-verify", "1"],
        CipherSuite12::SUPPORTED,
        ALL_GROUPS,
    )
    .unwrap_or_else(|e| panic!("an optional client certificate: {e:?}"));

    let required = run(
        &pki,
        &["-Verify", "1"],
        CipherSuite12::SUPPORTED,
        ALL_GROUPS,
    );
    assert!(
        required.is_err(),
        "a required client certificate cannot be satisfied with none"
    );
}

/// A required client certificate is satisfied: OpenSSL, run with `-Verify 1`
/// against the client's own CA, completes the handshake only if it received a
/// chain that verifies and a `CertificateVerify` that proves the key.
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_required_client_certificate_is_presented_and_accepted() {
    use rusty_tls::handrolled::client::ClientIdentity;
    use rusty_tls::handrolled::sign::SigningKey;

    for leaf in [Leaf::P256, Leaf::P384, Leaf::Rsa] {
        let server_pki = pki(Leaf::P256, SERVER);
        let client_pki = pki(leaf, "client.example");
        let key = match leaf {
            Leaf::P256 => SigningKey::ecdsa_p256(&client_pki.pkcs8),
            Leaf::P384 => SigningKey::ecdsa_p384(&client_pki.pkcs8),
            Leaf::Rsa => SigningKey::rsa(&client_pki.pkcs8),
        }
        .expect("the client key loads");
        let identity = ClientIdentity {
            certificates: &client_pki.chain,
            key: &key,
        };

        let client_root = std::env::temp_dir().join(format!(
            "rusty_tls_client12_clientca_{}_{}.pem",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&client_root, &client_pki.root_pem).expect("client CA");
        let server = serve(
            &server_pki,
            &["-Verify", "1", "-CAfile", &client_root.to_string_lossy()],
        );

        let anchors = anchors(&server_pki);
        let config = ClientConfig12 {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: ALL_GROUPS,
            cipher_suites: CipherSuite12::SUPPORTED,
            identity: Some(&identity),
        };
        let reply = talk(server.port, &config);
        let _ = std::fs::remove_file(&client_root);
        let reply = reply.unwrap_or_else(|e| panic!("{leaf:?}: {e:?}"));
        assert!(
            reply.response.starts_with("HTTP/1.0 200 ok"),
            "{leaf:?}: {}",
            reply.response
        );
    }
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_tls13_only_openssl_refuses_with_a_protocol_version_alert() {
    // `serve` passes -tls1_2 first; the later flag wins for the version range.
    let outcome = {
        let pki = pki(Leaf::P256, SERVER);
        let files = Files::new(&pki);
        let port = free_port();
        let mut child = Command::new("openssl")
            .args(["s_server", "-accept", &port.to_string()])
            .args([
                "-cert",
                &files.path("leaf.pem"),
                "-key",
                &files.path("key.pem"),
            ])
            .args(["-tls1_3", "-www", "-quiet"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("openssl");
        for _ in 0..100 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let anchors = anchors(&pki);
        let config = ClientConfig12 {
            server_name: ServerName::Dns(SERVER),
            anchors: &anchors,
            path: options(),
            groups: ALL_GROUPS,
            cipher_suites: CipherSuite12::SUPPORTED,
            identity: None,
        };
        let outcome = talk(port, &config);
        let _ = child.kill();
        let _ = child.wait();
        outcome
    };
    match outcome {
        Err(Talk::Refused(ClientError::PeerAlert(alert))) => {
            assert!(
                matches!(alert.description.0, 70 | 40),
                "protocol_version or handshake_failure: {alert:?}"
            );
        }
        other => panic!(
            "expected the peer's alert, got {:?}",
            other.map(|r| r.response)
        ),
    }
}

/// A certificate for another name is refused by the client's own check.
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_certificate_for_another_name_is_refused() {
    let pki = pki(Leaf::P256, "some.other.example");
    match run(&pki, &[], CipherSuite12::SUPPORTED, ALL_GROUPS) {
        Err(Talk::Refused(ClientError::Path(_))) => {}
        other => panic!("expected a path error, got {:?}", other.map(|r| r.response)),
    }
}
