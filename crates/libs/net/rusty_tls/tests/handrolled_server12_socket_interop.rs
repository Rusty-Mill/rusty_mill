//! The hand-rolled TLS 1.2 **server**, over a real socket, against `openssl
//! s_client` — stage 4b-iv.
//!
//! The rustls tests in `handrolled_server12.rs` hand whole records across as
//! byte slices. This file adds what they leave out, for the reasons the client
//! twin of this file gives: an unrelated implementation (OpenSSL, written by
//! other people from separate readings of the RFCs) and a real socket, which
//! delivers a header and a body separately.
//!
//! It also reaches what rustls cannot be made to do from a client:
//! a renegotiation request (`s_client`'s `R` command), which the server must
//! refuse with `no_renegotiation` and carry on; and a client that offers only
//! RSA PKCS#1 v1.5 signatures, which this server cannot produce and must
//! refuse rather than sign with something the client did not offer.
//!
//! `#[ignore]`d for the reason `handrolled_client12_socket_interop.rs` gives;
//! CI runs them with `-- --ignored`.
//!
//! ```text
//! cargo test --features handrolled-engine --test handrolled_server12_socket_interop \
//!     -- --ignored --nocapture
//! ```

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::PrivatePkcs8KeyDer;
use time::OffsetDateTime;

use rusty_tls::handrolled::client::ClientError;
use rusty_tls::handrolled::client12::{CipherSuite12, Incoming12};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::{ClientAuth, ServerError};
use rusty_tls::handrolled::server12::{ServerConfig12, ServerHandshake12};
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::x509::Certificate;

const SERVER: &str = "tls12.example";

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
    leaf_der: Vec<u8>,
    leaf_pem: String,
    key_pem: String,
    key_pkcs8: Vec<u8>,
}

fn pki(leaf: Leaf, name: &str) -> Pki {
    let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    root_params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("tls12 server socket test root".to_string()),
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
        leaf_der: leaf_cert.der().to_vec(),
        leaf_pem: leaf_cert.pem(),
        key_pem: leaf_key.serialize_pem(),
        key_pkcs8: leaf_key.serialize_der(),
    }
}

/// Files for `s_client`, removed on drop.
struct Files(PathBuf);

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl Files {
    fn new(entries: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rusty_tls_server12_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (name, content) in entries {
            std::fs::write(dir.join(name), content).expect("file");
        }
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

fn read_record(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    let mut record = header.to_vec();
    record.resize(5 + length, 0);
    stream.read_exact(&mut record[5..])?;
    Ok(record)
}

/// What the server side of a conversation saw.
#[derive(Debug)]
struct Served {
    suite: CipherSuite12,
    request: String,
    peer: Vec<Vec<u8>>,
}

#[derive(Debug)]
enum Failure {
    Refused(ServerError),
    /// The established connection failed (its error type is the client's).
    #[allow(dead_code)]
    Connection(ClientError),
    /// Carried so a test failure shows what happened.
    #[allow(dead_code)]
    Io(std::io::Error),
}

impl From<ServerError> for Failure {
    fn from(err: ServerError) -> Self {
        Self::Refused(err)
    }
}
impl From<ClientError> for Failure {
    fn from(err: ClientError) -> Self {
        Self::Connection(err)
    }
}
impl From<std::io::Error> for Failure {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Accept one connection and run the server over it: handshake, read the
/// request, answer, and close. A refusal's alert is written to the socket, as
/// a real caller would, so OpenSSL reports the reason.
fn serve_one(listener: &TcpListener, config: &ServerConfig12<'_>) -> Result<Served, Failure> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut socket = loop {
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return Err(err.into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(err.into()),
        }
    };
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;

    let mut handshake = ServerHandshake12::new(config)?;
    while !handshake.is_finished() {
        let record = read_record(&mut socket)?;
        match handshake.read_record(&record) {
            Ok(reply) if !reply.is_empty() => socket.write_all(&reply)?,
            Ok(_) => {}
            Err(error) => {
                if let Some(alert) = handshake.alert_record(&error) {
                    let _ = socket.write_all(&alert);
                }
                return Err(error.into());
            }
        }
    }
    let mut connection = handshake.into_connection()?;

    let mut request = Vec::new();
    while !request.ends_with(b"hello\n") {
        match connection.read(&read_record(&mut socket)?)? {
            Incoming12::Application(data) => request.extend_from_slice(&data),
            Incoming12::Reply(bytes) => socket.write_all(&bytes)?,
            Incoming12::Closed => break,
            _ => {}
        }
    }
    socket.write_all(&connection.write(b"ok\n")?)?;
    socket.write_all(&connection.close()?)?;
    Ok(Served {
        suite: connection.suite(),
        request: String::from_utf8_lossy(&request).into_owned(),
        peer: connection.peer_certificates().to_vec(),
    })
}

/// A server's material, and what to trust from clients.
struct Setup {
    certificates: Vec<Vec<u8>>,
    key: SigningKey,
    root_pem: String,
}

impl Setup {
    fn from(pki: &Pki, key: SigningKey) -> Self {
        Self {
            certificates: vec![pki.leaf_der.clone(), pki.root_der.clone()],
            key,
            root_pem: pki.root_pem.clone(),
        }
    }

    fn rcgen(leaf: Leaf) -> Self {
        let pki = pki(leaf, SERVER);
        let key = match leaf {
            Leaf::P256 => SigningKey::ecdsa_p256(&pki.key_pkcs8),
            Leaf::P384 => SigningKey::ecdsa_p384(&pki.key_pkcs8),
            Leaf::Rsa => SigningKey::rsa(&pki.key_pkcs8),
        }
        .expect("the leaf key loads");
        Self::from(&pki, key)
    }
}

/// Run `openssl s_client` against the server and return both verdicts.
///
/// `client_args` are added to a command line that verifies the name and the
/// chain and treats any verification failure as fatal, so a handshake that
/// completes is one OpenSSL accepted the server's certificate for.
fn converse(
    setup: &Setup,
    config: &ServerConfig12<'_>,
    client_args: &[&str],
    extra_files: &[(&str, &str)],
    stdin: &str,
) -> (Result<Served, Failure>, Output) {
    let mut entries = vec![("root.pem", setup.root_pem.as_str())];
    entries.extend_from_slice(extra_files);
    let files = Files::new(&entries);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let mut child = Command::new("openssl")
        .args(["s_client", "-connect", &format!("127.0.0.1:{port}")])
        .args(["-tls1_2", "-no_ticket", "-servername", SERVER])
        .args(["-verify_hostname", SERVER, "-verify_return_error"])
        .args(["-CAfile", &files.path("root.pem")])
        .args(client_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("openssl s_client starts");
    // Written up front; with `-quiet` the client waits for the server to close
    // rather than stopping when its input ends.
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");

    let served = serve_one(&listener, config);
    drop(listener);
    // Give the client time to read the close, then collect it.
    let output = child.wait_with_output().expect("s_client finishes");
    (served, output)
}

fn the_client_said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn config<'a>(
    setup: &'a Setup,
    suites: &'a [CipherSuite12],
    groups: &'a [NamedGroup],
    client_auth: Option<&'a ClientAuth<'a>>,
) -> ServerConfig12<'a> {
    ServerConfig12 {
        certificates: &setup.certificates,
        key: &setup.key,
        cipher_suites: suites,
        groups,
        client_auth,
        alpn: &[],
    }
}

const ALL_GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

/// A conversation that must have completed on both sides.
fn completes(
    setup: &Setup,
    config: &ServerConfig12<'_>,
    client_args: &[&str],
    extra_files: &[(&str, &str)],
) -> Served {
    let (served, output) = converse(setup, config, client_args, extra_files, "hello\n");
    let said = the_client_said(&output);
    let served = served.unwrap_or_else(|f| panic!("server: {f:?}\nopenssl said:\n{said}"));
    assert!(output.status.success(), "s_client failed:\n{said}");
    assert!(
        said.contains("ok"),
        "no response reached the client:\n{said}"
    );
    assert_eq!(served.request, "hello\n");
    served
}

// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn every_suite_completes_with_openssl_and_is_the_one_negotiated() {
    let ecdsa = [
        (
            "ECDHE-ECDSA-AES128-GCM-SHA256",
            CipherSuite12::ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
        ),
        (
            "ECDHE-ECDSA-AES256-GCM-SHA384",
            CipherSuite12::ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
        ),
        (
            "ECDHE-ECDSA-CHACHA20-POLY1305",
            CipherSuite12::ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
        ),
    ];
    let rsa = [
        (
            "ECDHE-RSA-AES128-GCM-SHA256",
            CipherSuite12::ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        ),
        (
            "ECDHE-RSA-AES256-GCM-SHA384",
            CipherSuite12::ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        ),
        (
            "ECDHE-RSA-CHACHA20-POLY1305",
            CipherSuite12::ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
        ),
    ];
    for (leaf, suites) in [
        (Leaf::P256, &ecdsa),
        (Leaf::P384, &ecdsa),
        (Leaf::Rsa, &rsa),
    ] {
        let setup = Setup::rcgen(leaf);
        for (name, suite) in suites {
            let config = config(&setup, CipherSuite12::SUPPORTED, ALL_GROUPS, None);
            let served = completes(&setup, &config, &["-quiet", "-cipher", name], &[]);
            assert_eq!(served.suite, *suite, "{leaf:?} {name}");
        }
    }
}

/// The server is configured with one group, and the client offers that group
/// plus the certificate's own curve (RFC 8422 §5.1), so each group is the one
/// that must have been used.
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn every_group_completes_with_openssl() {
    for (group, name) in [
        (NamedGroup::X25519, "X25519"),
        (NamedGroup::SecP256R1, "P-256"),
        (NamedGroup::SecP384R1, "P-384"),
    ] {
        for (leaf, curve) in [
            (Leaf::P256, "P-256"),
            (Leaf::P384, "P-384"),
            (Leaf::Rsa, "P-256"),
        ] {
            let setup = Setup::rcgen(leaf);
            let groups = [group];
            let config = config(&setup, CipherSuite12::SUPPORTED, &groups, None);
            let offered = if name == curve {
                name.to_string()
            } else {
                format!("{name}:{curve}")
            };
            completes(&setup, &config, &["-quiet", "-groups", &offered], &[]);
        }
    }
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn an_ecdsa_key_whose_curve_the_client_did_not_offer_is_refused() {
    let setup = Setup::rcgen(Leaf::P256);
    let config = config(&setup, CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    let (served, output) = converse(
        &setup,
        &config,
        &["-quiet", "-groups", "X25519"],
        &[],
        "hello\n",
    );
    assert!(
        matches!(
            served,
            Err(Failure::Refused(ServerError::NoSharedSignatureScheme))
        ),
        "{served:?}"
    );
    assert!(!output.status.success());
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn rsa_pss_signatures_are_accepted_and_pkcs1_only_clients_are_refused() {
    let setup = Setup::rcgen(Leaf::Rsa);
    let config = config(&setup, CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    for pss in [
        "rsa_pss_rsae_sha256",
        "rsa_pss_rsae_sha384",
        "rsa_pss_rsae_sha512",
    ] {
        completes(&setup, &config, &["-quiet", "-sigalgs", pss], &[]);
    }

    // This server's key signs PSS only. A client that will take nothing else
    // is refused, never answered with a signature it did not offer.
    let (served, output) = converse(
        &setup,
        &config,
        &["-quiet", "-sigalgs", "RSA+SHA256"],
        &[],
        "hello\n",
    );
    assert!(
        matches!(
            served,
            Err(Failure::Refused(ServerError::NoSharedSignatureScheme))
        ),
        "{served:?}"
    );
    assert!(!output.status.success());
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn ed25519_is_signed_and_accepted_by_openssl() {
    let (pki, key) = ed25519_via_openssl();
    let setup = Setup::from(&pki, key);
    // An Ed25519 key authenticates ECDHE_ECDSA suites; only the leaf is sent
    // since the certificate is its own root.
    let setup = Setup {
        certificates: vec![pki.leaf_der.clone()],
        ..setup
    };
    let config = config(&setup, CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    completes(&setup, &config, &["-quiet"], &[]);
}

/// A self-signed Ed25519 certificate made by OpenSSL, as in the client twin of
/// this file: rcgen's Ed25519 keys are PKCS#8 v2, which OpenSSL 3.0 refuses.
fn ed25519_via_openssl() -> (Pki, SigningKey) {
    let files = Files::new(&[]);
    let (key, cert) = (files.path("key.pem"), files.path("cert.pem"));
    let status = Command::new("openssl")
        .args([
            "req", "-x509", "-newkey", "ed25519", "-nodes", "-days", "30",
        ])
        .args(["-subj", &format!("/CN={SERVER}")])
        .args(["-addext", &format!("subjectAltName=DNS:{SERVER}")])
        .args(["-keyout", &key, "-out", &cert])
        .stderr(Stdio::null())
        .status()
        .expect("openssl req");
    assert!(
        status.success(),
        "openssl could not make an Ed25519 certificate"
    );
    let der = |args: &[&str]| {
        Command::new("openssl")
            .args(args)
            .output()
            .expect("openssl")
            .stdout
    };
    let leaf_der = der(&["x509", "-in", &cert, "-outform", "DER"]);
    let key_pkcs8 = der(&["pkey", "-in", &key, "-outform", "DER"]);
    let pem = std::fs::read_to_string(&cert).expect("cert");
    let signing = SigningKey::ed25519(&key_pkcs8).expect("the Ed25519 key loads");
    (
        Pki {
            root_der: leaf_der.clone(),
            root_pem: pem.clone(),
            leaf_der,
            leaf_pem: pem,
            key_pem: String::new(),
            key_pkcs8,
        },
        signing,
    )
}

// ---------------------------------------------------------------------------
// Client authentication
// ---------------------------------------------------------------------------

fn client_auth_anchor(client: &Pki) -> Vec<u8> {
    client.root_der.clone()
}

fn auth_config<'a>(setup: &'a Setup, auth: &'a ClientAuth<'a>) -> ServerConfig12<'a> {
    config(setup, CipherSuite12::SUPPORTED, ALL_GROUPS, Some(auth))
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_client_certificate_from_openssl_is_verified_and_reported() {
    for leaf in [Leaf::P256, Leaf::P384, Leaf::Rsa] {
        let setup = Setup::rcgen(Leaf::P256);
        let client = pki(leaf, "client.example");
        let root = client_auth_anchor(&client);
        let anchors = [TrustAnchor::from_certificate(
            &Certificate::parse(&root).expect("root"),
        )];
        let auth = ClientAuth {
            anchors: &anchors,
            path: options(),
            required: true,
        };
        let config = auth_config(&setup, &auth);

        let files = Files::new(&[
            ("client.pem", &client.leaf_pem),
            ("client.key", &client.key_pem),
        ]);
        let served = completes(
            &setup,
            &config,
            &[
                "-quiet",
                "-cert",
                &files.path("client.pem"),
                "-key",
                &files.path("client.key"),
            ],
            &[
                ("client.pem", &client.leaf_pem),
                ("client.key", &client.key_pem),
            ],
        );
        assert_eq!(
            served.peer.first(),
            Some(&client.leaf_der),
            "{leaf:?} client"
        );
    }
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_client_with_no_certificate_is_refused_when_required() {
    let setup = Setup::rcgen(Leaf::P256);
    let client = pki(Leaf::P256, "client.example");
    let root = client_auth_anchor(&client);
    let anchors = [TrustAnchor::from_certificate(
        &Certificate::parse(&root).expect("root"),
    )];

    let required = ClientAuth {
        anchors: &anchors,
        path: options(),
        required: true,
    };
    let (served, output) = converse(
        &setup,
        &auth_config(&setup, &required),
        &["-quiet"],
        &[],
        "hello\n",
    );
    assert!(
        matches!(
            served,
            Err(Failure::Refused(ServerError::ClientCertificateRequired))
        ),
        "{served:?}"
    );
    assert!(!output.status.success());

    let optional = ClientAuth {
        anchors: &anchors,
        path: options(),
        required: false,
    };
    let served = completes(&setup, &auth_config(&setup, &optional), &["-quiet"], &[]);
    assert!(served.peer.is_empty());
}

// ---------------------------------------------------------------------------
// Renegotiation
// ---------------------------------------------------------------------------

/// `s_client`'s `R` command starts a renegotiation: a ClientHello inside the
/// encrypted connection. RFC 5746 says to refuse it with a `no_renegotiation`
/// warning. OpenSSL, which asked, treats the refusal as fatal on its own side
/// and says so by name — which is the evidence that the alert it received was
/// exactly `no_renegotiation(100)`, and that the server read the request as a
/// renegotiation and not as garbage (which would have been a fatal alert from
/// the server, and a different message).
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_renegotiation_request_is_refused_with_no_renegotiation() {
    let setup = Setup::rcgen(Leaf::P256);
    let config = config(&setup, CipherSuite12::SUPPORTED, ALL_GROUPS, None);
    // No `-quiet`: interactive commands are off in quiet mode.
    let (served, output) = converse(&setup, &config, &[], &[], "R\n");
    let said = the_client_said(&output);
    assert!(
        said.contains("RENEGOTIATING"),
        "the client never asked:\n{said}"
    );
    assert!(
        said.contains("no renegotiation"),
        "not refused by name:\n{said}"
    );
    // Not a protocol failure on the server's side: the refusal was its own
    // choice, and what ended the conversation was the peer going away.
    assert!(!matches!(served, Err(Failure::Refused(_))), "{served:?}");
}
