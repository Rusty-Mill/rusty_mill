//! Version negotiation over a real socket against OpenSSL — stage 4b-v.
//!
//! `handrolled_negotiate.rs` pits the two-version endpoints against rustls.
//! OpenSSL is a second, unrelated opinion on the same questions: which version
//! does a client that offers both get, what does it make of the `DOWNGRD`
//! sentinel in a 1.2 `ServerHello`, and does `TLS_FALLBACK_SCSV` (which only
//! OpenSSL can be made to send) get the `inappropriate_fallback` it should.
//!
//! `#[ignore]`d for the reason `handrolled_client12_socket_interop.rs` gives;
//! CI runs them with `-- --ignored`.
//!
//! ```text
//! cargo test --features handrolled-engine --test handrolled_negotiate_socket_interop \
//!     -- --ignored --nocapture
//! ```

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use time::OffsetDateTime;

use rusty_tls::handrolled::client::{CipherSuite, ClientConfig, ClientError, Incoming};
use rusty_tls::handrolled::client12::{CipherSuite12, Incoming12};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, Established, ServerConfigBoth, ServerHandshakeBoth,
    Version,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::{ServerConfig, ServerError};
use rusty_tls::handrolled::server12::ServerConfig12;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaf {
    P256,
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

/// What our server saw of a conversation.
#[derive(Debug)]
struct Served {
    version: Version,
    request: String,
}

#[derive(Debug)]
enum Failure {
    Refused(ServerError),
    /// Carried so a test failure shows what happened.
    #[allow(dead_code)]
    Connection(ClientError),
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

struct Material {
    pki: Pki,
    certificates: Vec<Vec<u8>>,
    key: SigningKey,
}

impl Material {
    fn new() -> Self {
        let pki = pki(Leaf::P256, SERVER);
        let certificates = vec![pki.leaf_der.clone(), pki.root_der.clone()];
        let key = SigningKey::ecdsa_p256(&pki.key_pkcs8).expect("the leaf key loads");
        Self {
            pki,
            certificates,
            key,
        }
    }
}

const ALL_GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

fn read_app(established: &mut Established, record: &[u8]) -> Result<Option<Vec<u8>>, Failure> {
    Ok(match established {
        Established::Tls13(c) => match c.read(record)? {
            Incoming::Application(data) => Some(data),
            Incoming::Reply(_) | Incoming::Closed => None,
            _ => Some(Vec::new()),
        },
        Established::Tls12(c) => match c.read(record)? {
            Incoming12::Application(data) => Some(data),
            Incoming12::Closed => None,
            _ => Some(Vec::new()),
        },
        _ => unreachable!("only two versions"),
    })
}

fn write_app(established: &mut Established, data: &[u8]) -> Result<Vec<u8>, Failure> {
    Ok(match established {
        Established::Tls13(c) => c.write(data)?,
        Established::Tls12(c) => c.write(data)?,
        _ => unreachable!("only two versions"),
    })
}

/// Accept one connection and run the two-version server over it. A refusal's
/// alert is written to the socket, as a real caller would.
fn serve_one(listener: &TcpListener, config: &ServerConfigBoth<'_>) -> Result<Served, Failure> {
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

    let mut handshake = ServerHandshakeBoth::new(config);
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
    let version = handshake.version().expect("finished");
    let mut connection = handshake.into_connection()?;

    let mut request = Vec::new();
    while !request.ends_with(b"hello\n") {
        match read_app(&mut connection, &read_record(&mut socket)?)? {
            Some(data) => request.extend_from_slice(&data),
            None => break,
        }
    }
    socket.write_all(&write_app(&mut connection, b"ok\n")?)?;
    let closing = match &mut connection {
        Established::Tls13(c) => c.close()?,
        Established::Tls12(c) => c.close()?,
        _ => unreachable!("only two versions"),
    };
    socket.write_all(&closing)?;
    Ok(Served {
        version,
        request: String::from_utf8_lossy(&request).into_owned(),
    })
}

fn the_client_said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Run `openssl s_client` against our server.
fn s_client(material: &Material, args: &[&str]) -> (Result<Served, Failure>, Output) {
    let files = Files::new(&[("root.pem", material.pki.root_pem.as_str())]);
    let tls13 = ServerConfig {
        certificates: &material.certificates,
        key: &material.key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: ALL_GROUPS,
        client_auth: None,
        tickets: None,
    };
    let tls12 = ServerConfig12 {
        certificates: &material.certificates,
        key: &material.key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: ALL_GROUPS,
        client_auth: None,
    };
    let config = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let mut child = Command::new("openssl")
        .args(["s_client", "-connect", &format!("127.0.0.1:{port}")])
        .args(["-servername", SERVER, "-verify_hostname", SERVER])
        .args(["-verify_return_error", "-quiet", "-no_ticket"])
        .args(["-CAfile", &files.path("root.pem")])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("openssl s_client starts");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"hello\n")
        .expect("write stdin");
    let served = serve_one(&listener, &config);
    drop(listener);
    (served, child.wait_with_output().expect("s_client finishes"))
}

fn completes_with_s_client(material: &Material, args: &[&str]) -> Version {
    let (served, output) = s_client(material, args);
    let said = the_client_said(&output);
    let served = served.unwrap_or_else(|f| panic!("server: {f:?}\nopenssl said:\n{said}"));
    assert!(output.status.success(), "s_client failed:\n{said}");
    assert!(
        said.contains("ok"),
        "no response reached the client:\n{said}"
    );
    assert_eq!(served.request, "hello\n");
    served.version
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn openssl_gets_the_version_it_asks_for_and_1_3_when_it_does_not() {
    let material = Material::new();
    assert_eq!(completes_with_s_client(&material, &[]), Version::Tls13);
    assert_eq!(
        completes_with_s_client(&material, &["-tls1_3"]),
        Version::Tls13
    );
    // A TLS 1.2-only OpenSSL client is answered in 1.2 by a server that also
    // speaks 1.3, with the sentinel in `random`. OpenSSL, which cannot be
    // attacked into 1.2 it never offered 1.3 for, must carry on.
    assert_eq!(
        completes_with_s_client(&material, &["-tls1_2"]),
        Version::Tls12
    );
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_real_fallback_scsv_is_refused_unless_1_3_was_offered() {
    let material = Material::new();

    let (served, output) = s_client(&material, &["-tls1_2", "-fallback_scsv"]);
    let said = the_client_said(&output);
    assert!(
        matches!(
            served,
            Err(Failure::Refused(ServerError::InappropriateFallback))
        ),
        "{served:?}\n{said}"
    );
    assert!(!output.status.success());
    assert!(
        said.to_lowercase().contains("inappropriate fallback"),
        "OpenSSL did not receive inappropriate_fallback by name:\n{said}"
    );

    // Offering 1.3 as well, the same flag is no fallback at all.
    assert_eq!(
        completes_with_s_client(&material, &["-fallback_scsv"]),
        Version::Tls13
    );
}

// ---------------------------------------------------------------------------
// Our client, against `openssl s_server`
// ---------------------------------------------------------------------------

struct OpensslServer {
    child: Child,
    port: u16,
    _files: Files,
    /// Held open so `s_server` never meets a closed pipe when it logs.
    _stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Drop for OpensslServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn s_server(material: &Material, extra: &[&str]) -> OpensslServer {
    let files = Files::new(&[
        ("leaf.pem", material.pki.leaf_pem.as_str()),
        ("key.pem", material.pki.key_pem.as_str()),
        ("root.pem", material.pki.root_pem.as_str()),
    ]);
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port();
    // Without `-quiet`, `s_server` prints `ACCEPT` once it is listening. That
    // is the readiness signal: probing with a connection would be a handshake
    // that never completes, which TLS 1.3 `s_server` does not always survive.
    let mut child = Command::new("openssl")
        .args(["s_server", "-accept", &port.to_string()])
        .args(["-cert", &files.path("leaf.pem")])
        .args(["-cert_chain", &files.path("root.pem")])
        .args(["-key", &files.path("key.pem")])
        .args(["-no_ticket", "-www"])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("openssl s_server starts");
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("stdout"));
    let mut line = String::new();
    loop {
        line.clear();
        let n = std::io::BufRead::read_line(&mut stdout, &mut line).expect("s_server output");
        assert!(n > 0, "openssl s_server exited before listening: {extra:?}");
        if line.starts_with("ACCEPT") {
            break;
        }
    }
    OpensslServer {
        child,
        port,
        _files: files,
        _stdout: stdout,
    }
}

/// A connection that copies bytes both ways and rewrites the first record the
/// client sends: an attacker who can alter a message but not break the
/// connection.
fn proxy(to: u16, rewrite: fn(&[u8]) -> Vec<u8>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let Ok((mut client, _)) = listener.accept() else {
            return;
        };
        let Ok(mut server) = TcpStream::connect(("127.0.0.1", to)) else {
            return;
        };
        let (mut client_r, mut server_w) = (
            client.try_clone().expect("clone"),
            server.try_clone().expect("clone"),
        );
        std::thread::spawn(move || {
            if let Ok(first) = read_record(&mut client_r) {
                let _ = server_w.write_all(&rewrite(&first));
                let mut buf = [0u8; 4096];
                while let Ok(n) = client_r.read(&mut buf) {
                    if n == 0 || server_w.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        });
        let mut buf = [0u8; 4096];
        while let Ok(n) = server.read(&mut buf) {
            if n == 0 || client.write_all(&buf[..n]).is_err() {
                break;
            }
        }
    });
    port
}

fn strip_tls13(record: &[u8]) -> Vec<u8> {
    use rusty_tls::handrolled::handshake::{extension, messages, Extension, HandshakeType};
    use rusty_tls::handrolled::handshake12::{message, ClientHello12};
    let parsed = messages(&record[5..]).expect("a hello");
    let hello = ClientHello12::parse(parsed[0].body).expect("parses");
    let owned: Vec<(u16, Vec<u8>)> = hello
        .extensions
        .iter()
        .map(|e| {
            if e.typ == extension::SUPPORTED_VERSIONS {
                (e.typ, vec![2, 3, 3])
            } else {
                (e.typ, e.data.to_vec())
            }
        })
        .collect();
    let extensions = owned
        .iter()
        .map(|(typ, data)| Extension { typ: *typ, data })
        .collect();
    let body = ClientHello12 {
        extensions,
        cipher_suites: hello.cipher_suites.clone(),
        ..hello
    }
    .encode();
    let fragment = message(HandshakeType::ClientHello, &body);
    let mut out = vec![22, 3, 3];
    out.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
    out.extend_from_slice(&fragment);
    out
}

/// Handshake with the server at `port`, send a request, read the response to
/// the close.
fn talk(material: &Material, port: u16) -> Result<(Version, String), Failure> {
    let anchors = [TrustAnchor::from_certificate(
        &Certificate::parse(&material.pki.root_der).expect("root parses"),
    )];
    let tls13 = ClientConfig {
        server_name: ServerName::Dns(SERVER),
        anchors: &anchors,
        path: options(),
        groups: ALL_GROUPS,
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };
    let config = ClientConfigBoth::new(tls13, CipherSuite12::SUPPORTED);
    let mut socket = TcpStream::connect(("127.0.0.1", port))?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;

    let (mut handshake, hello) = ClientHandshakeBoth::start(&config)?;
    socket.write_all(&hello)?;
    while !handshake.is_finished() {
        let reply = handshake.read_record(&read_record(&mut socket)?)?;
        if !reply.is_empty() {
            socket.write_all(&reply)?;
        }
    }
    let version = handshake.version().expect("finished");
    let mut connection = handshake.into_connection()?;
    socket.write_all(&write_app(&mut connection, b"GET / HTTP/1.0\r\n\r\n")?)?;

    let mut body = Vec::new();
    while let Ok(record) = read_record(&mut socket) {
        match read_app(&mut connection, &record)? {
            Some(data) => body.extend_from_slice(&data),
            None => break,
        }
    }
    Ok((version, String::from_utf8_lossy(&body).into_owned()))
}

#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn our_client_gets_the_best_version_openssl_offers() {
    let material = Material::new();
    for (extra, want) in [
        (&[][..], Version::Tls13),
        (&["-tls1_3"][..], Version::Tls13),
        // OpenSSL capped at 1.2 cannot speak 1.3 and so writes no sentinel; a
        // client that offered both must accept that.
        (&["-tls1_2"][..], Version::Tls12),
    ] {
        let server = s_server(&material, extra);
        let (version, response) =
            talk(&material, server.port).unwrap_or_else(|f| panic!("{extra:?}: {f:?}"));
        assert_eq!(version, want, "{extra:?}");
        assert!(
            response.starts_with("HTTP/1.0 200 ok"),
            "{extra:?}: {response}"
        );
    }
}

/// OpenSSL is the 1.3-capable server that writes the sentinel: with TLS 1.3
/// removed from the hello in flight it answers in 1.2 and says so in `random`.
/// The client that offered 1.3 must refuse.
#[test]
#[ignore = "needs the openssl binary; CI runs it with --ignored"]
fn a_hello_stripped_in_flight_is_caught_by_the_sentinel_openssl_writes() {
    let material = Material::new();
    let server = s_server(&material, &[]);
    let through = proxy(server.port, strip_tls13);
    match talk(&material, through) {
        Err(Failure::Connection(ClientError::DowngradeDetected)) => {}
        other => panic!("expected DowngradeDetected, got {other:?}"),
    }
}
