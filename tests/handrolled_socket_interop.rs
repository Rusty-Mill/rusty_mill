//! The hand-rolled **server**, over a real socket, driven by a client nobody
//! here wrote — `rusty_tls#45`.
//!
//! Stage 5 shipped the server with 19 tests, and every one of them is `rustls`
//! the client, in memory, with whole records handed across as byte slices. Two
//! separate things were missing, and they fail differently:
//!
//! 1. **One implementation, not an independent one.** `rustls` agreeing with
//!    this code is weaker evidence than an unrelated stack agreeing with it:
//!    where both derive from the same reading of RFC 8446, a shared misreading
//!    agrees with itself. OpenSSL's TLS 1.3 was written by different people
//!    from a different reading, which is the whole point.
//! 2. **No socket.** An in-memory harness hands over whole records. A socket
//!    does not — it delivers a header and a body that may arrive in separate
//!    reads, and the caller has to reassemble before the server sees anything.
//!    The client half already caught one bug of exactly this shape: its
//!    reassembly buffer was carried by tests that never made it reassemble.
//!
//! The client side has `handrolled_interop` for this. The server had no
//! counterpart until now.
//!
//! # Why these are `#[ignore]`d
//!
//! Not for the reason the client's interop suite is. **These tests are
//! hermetic** — a loopback listener and a child process, no network, no DNS,
//! nothing that can be reconfigured out from under them.
//!
//! What they depend on is the `openssl` **binary** being installed, and on its
//! version speaking TLS 1.3 (3.x here; 1.1.1 and later do). That is not
//! something this repo controls, and a suite that fails on a machine without
//! OpenSSL is reporting a fact about the machine.
//!
//! `#[ignore]` rather than detecting `openssl` and skipping, for the reason
//! this repo has settled on twice already: a test that quietly passes when its
//! precondition is absent reports `ok` for a run that did nothing. An ignored
//! test reports `ignored`, which is the truth. The same reasoning is written
//! out at greater length in `handrolled_interop`.
//!
//! ```text
//! cargo test --features handrolled-engine --test handrolled_socket_interop \
//!     -- --ignored --nocapture
//! ```
//!
//! **CI runs them anyway**, as an explicit `-- --ignored` step, because the
//! runner does have OpenSSL and these tests need nothing else. `#[ignore]` here
//! means "cannot be assumed to work everywhere", not "never runs" — the latter
//! would leave the only independent check of this server permanently
//! unexercised, which is a worse outcome than the one it avoids. The CI step
//! prints `openssl version` first so that the binary disappearing fails the
//! job instead of silently running nothing.
//!
//! # What a pass here proves that the in-memory tests do not
//!
//! - An **independent** TLS 1.3 client completes a handshake with this server.
//! - The server's records survive a real socket, where a record arrives in
//!   however many pieces the kernel felt like.
//! - The chain this server serialises is one a third-party client will
//!   *verify* — `-verify_return_error` and `-verify_hostname` mean OpenSSL
//!   exits non-zero rather than warning, so "it connected" cannot stand in for
//!   "it was trusted".
//! - Application data flows both ways afterwards, and `close_notify` is seen
//!   as an orderly close rather than an error.
//!
//! # What it deliberately does not do
//!
//! It does not disable verification. Pointing OpenSSL at the generated root
//! with `-CAfile` costs one temporary file and keeps the most valuable
//! assertion in the suite; `-noverify` would turn every one of these into a
//! test that the bytes parse.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};

use rusty_tls::handrolled::client::{CipherSuite, Incoming};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::server::{ServerConfig, ServerHandshake};
use rusty_tls::handrolled::sign::SigningKey;

const SERVER_NAME: &str = "handrolled.example";
/// Long enough that a slow machine is not a failure, short enough that a
/// genuine hang is reported as one rather than hanging the suite.
const TIMEOUT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// Certificates
// ---------------------------------------------------------------------------

struct Pki {
    chain: Vec<Vec<u8>>,
    key_pkcs8: Vec<u8>,
    root_pem: String,
}

/// A root and a leaf for [`SERVER_NAME`].
///
/// `keyCertSign` on the root and an explicit validity window, for the same
/// reasons the rest of the suite sets them: a CA without the key usage is one
/// a strict validator should refuse, and `rcgen`'s default `not_after` is the
/// year 4096.
fn pki() -> Pki {
    let root_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("root params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    root_params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("handrolled socket-interop root".to_string()),
    );
    dated(&mut root_params);
    let root = root_params.self_signed(&root_key).expect("root");

    let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("leaf key");
    let mut leaf_params =
        CertificateParams::new(vec![SERVER_NAME.to_string()]).expect("leaf params");
    dated(&mut leaf_params);
    let leaf = leaf_params
        .signed_by(&leaf_key, &root, &root_key)
        .expect("leaf");

    Pki {
        chain: vec![leaf.der().to_vec(), root.der().to_vec()],
        key_pkcs8: leaf_key.serialize_der(),
        root_pem: root.pem(),
    }
}

/// An explicit validity window on every certificate.
///
/// `rcgen` defaults `not_after` to the year 4096. OpenSSL would accept that
/// happily, so nothing here fails without this — but a certificate valid for
/// two thousand years is not the thing being tested, and this suite has a
/// sibling where relying on that default produced a green "expired
/// certificate" test for a certificate that was still perfectly valid.
///
/// Ends before 2050 so the dates stay `UTCTime`, matching every other
/// certificate this repo generates.
fn dated(params: &mut CertificateParams) {
    params.not_before =
        time::OffsetDateTime::from_unix_timestamp(1_577_836_800).expect("2020-01-01");
    params.not_after =
        time::OffsetDateTime::from_unix_timestamp(2_366_841_600).expect("2045-01-01");
}

/// The root, on disk, for OpenSSL's `-CAfile`.
///
/// Removed on drop so a failing run does not leave certificates in the
/// temporary directory.
struct RootFile(std::path::PathBuf);

impl RootFile {
    fn write(pem: &str, tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rusty_tls-interop-{}-{tag}.pem",
            std::process::id()
        ));
        std::fs::write(&path, pem).expect("write the root");
        Self(path)
    }
}

impl Drop for RootFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// ---------------------------------------------------------------------------
// The socket half
// ---------------------------------------------------------------------------

/// Read exactly one TLS record.
///
/// **This is the part an in-memory harness cannot exercise.** The header names
/// the body's length, and neither the header nor the body is guaranteed to
/// arrive in one `read` — `read_exact` is what turns a byte stream back into
/// records, and getting it wrong is invisible until a record happens to be
/// split.
fn read_record(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let length = u16::from_be_bytes([header[3], header[4]]) as usize;
    let mut record = Vec::with_capacity(5 + length);
    record.extend_from_slice(&header);
    record.resize(5 + length, 0);
    stream.read_exact(&mut record[5..])?;
    Ok(record)
}

/// What the server observed, reported back to the test.
#[derive(Debug, Default)]
struct Report {
    handshake_completed: bool,
    echoed: Option<Vec<u8>>,
    closed_cleanly: bool,
    /// Records the client sent during the handshake — reported rather than
    /// asserted, exactly as the client-side interop suite reports flight
    /// counts. Asserting a number would be asserting something about OpenSSL.
    handshake_records: usize,
    error: Option<String>,
}

/// Accept one connection, complete a handshake, echo one message, and observe
/// the close.
fn serve(listener: TcpListener, pki: Pki) -> Report {
    let mut report = Report::default();

    let key = match SigningKey::ecdsa_p256(&pki.key_pkcs8) {
        Ok(key) => key,
        Err(err) => {
            report.error = Some(format!("the signing key was refused: {err}"));
            return report;
        }
    };
    let config = ServerConfig {
        certificates: &pki.chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &[NamedGroup::X25519, NamedGroup::SecP256R1],
        // OpenSSL is not asked to authenticate here; that path has its own
        // hermetic coverage in `handrolled_server`.
        client_auth: None,
    };

    let Ok((mut socket, _)) = listener.accept() else {
        report.error = Some("no connection arrived".to_string());
        return report;
    };
    let _ = socket.set_read_timeout(Some(TIMEOUT));
    let _ = socket.set_write_timeout(Some(TIMEOUT));

    let mut handshake = ServerHandshake::new(&config);
    while !handshake.is_finished() {
        let record = match read_record(&mut socket) {
            Ok(record) => record,
            Err(err) => {
                report.error = Some(format!("reading a handshake record: {err}"));
                return report;
            }
        };
        report.handshake_records += 1;
        match handshake.read_record(&record) {
            Ok(reply) => {
                if !reply.is_empty() && socket.write_all(&reply).is_err() {
                    report.error = Some("writing the server flight".to_string());
                    return report;
                }
            }
            Err(err) => {
                // Send the alert before giving up: a peer told why it was
                // refused can report something useful, and this is the path a
                // real deployment takes.
                if let Some(alert) = handshake.alert_record(&err) {
                    let _ = socket.write_all(&alert);
                }
                report.error = Some(format!("the handshake failed: {err}"));
                return report;
            }
        }
    }

    let mut connection = match handshake.into_connection() {
        Ok(connection) => connection,
        Err(err) => {
            report.error = Some(format!("into_connection: {err}"));
            return report;
        }
    };
    report.handshake_completed = true;

    // Echo the first application message, then watch for the close. Anything
    // else the peer sends post-handshake is tolerated rather than treated as
    // data — that distinction is the one a vacuous session-ticket test missed
    // in stage 3c-ii.
    loop {
        let record = match read_record(&mut socket) {
            Ok(record) => record,
            Err(_) => return report,
        };
        match connection.read(&record) {
            Ok(Incoming::Application(data)) if report.echoed.is_none() => {
                let Ok(out) = connection.write(&data) else {
                    report.error = Some("sealing the echo failed".to_string());
                    return report;
                };
                if socket.write_all(&out).is_err() {
                    report.error = Some("writing the echo failed".to_string());
                    return report;
                }
                report.echoed = Some(data);
            }
            Ok(Incoming::Application(_)) => {}
            Ok(Incoming::Handled) => {}
            Ok(Incoming::Reply(bytes)) => {
                let _ = socket.write_all(&bytes);
            }
            Ok(Incoming::Closed) => {
                report.closed_cleanly = true;
                return report;
            }
            // `Incoming` is `#[non_exhaustive]`. Tolerating a variant that did
            // not exist when this was written is right for an interop harness
            // — the alternative is a suite that fails the day the enum grows,
            // for a reason that has nothing to do with interop.
            Ok(_) => {}
            Err(err) => {
                report.error = Some(format!("reading application data: {err}"));
                return report;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The OpenSSL half
// ---------------------------------------------------------------------------

/// Start `openssl s_client` against `port`, verifying properly.
///
/// The verification flags are the point of the whole suite, so they are worth
/// naming individually:
///
/// - `-CAfile` — trust the generated root, and nothing else.
/// - `-verify_return_error` — a verification failure ends the connection with
///   a non-zero exit rather than a printed warning. Without it OpenSSL
///   connects anyway and the test would pass on a chain it rejected.
/// - `-verify_hostname` — the certificate must actually be for [`SERVER_NAME`],
///   which is not what we dialled. Dialling `127.0.0.1` and checking the name
///   separately is what makes this a test of the certificate rather than of
///   the socket.
/// - `-tls1_3` — refuse to fall back, so a failure is a failure.
fn s_client(port: u16, ca_file: &std::path::Path) -> std::io::Result<Child> {
    Command::new("openssl")
        .arg("s_client")
        .arg("-connect")
        .arg(format!("127.0.0.1:{port}"))
        .arg("-servername")
        .arg(SERVER_NAME)
        .arg("-CAfile")
        .arg(ca_file)
        .arg("-verify_hostname")
        .arg(SERVER_NAME)
        .arg("-verify_return_error")
        .arg("-tls1_3")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Everything the two halves observed about one connection.
struct Session {
    report: Report,
    stdout: String,
    stderr: String,
    status: std::process::ExitStatus,
}

/// Run one connection end to end: our server on a thread, OpenSSL as a child.
///
/// `probe` is written to OpenSSL's stdin and expected back, which is how the
/// echo is observed from the far side rather than only from ours.
fn session(probe: &str, tag: &str) -> Session {
    let pki = pki();
    let root = RootFile::write(&pki.root_pem, tag);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(serve(listener, pki));
    });

    let mut child = s_client(port, &root.0).expect(
        "`openssl` must be on PATH — these tests are #[ignore]d precisely \
         because that cannot be assumed",
    );

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");

    // Read OpenSSL's output on its own thread. Its banner is written before
    // our probe comes back, and a single-threaded read-then-write would
    // deadlock against a full pipe buffer.
    let (out_tx, out_rx) = mpsc::channel();
    let echo_marker = probe.trim().to_string();
    let reader = thread::spawn(move || {
        let mut lines = Vec::new();
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let seen = line.trim() == echo_marker;
                    lines.push(line);
                    if seen {
                        // Tell the main thread the echo arrived; keep reading
                        // so the pipe never fills.
                        let _ = out_tx.send(());
                    }
                }
            }
        }
        lines.join("")
    });

    let _ = stdin.write_all(probe.as_bytes());
    let _ = stdin.flush();

    // Wait for the echo, then close stdin so `s_client` sends close_notify.
    let echoed = out_rx.recv_timeout(TIMEOUT).is_ok();
    drop(stdin);

    let report = rx.recv_timeout(TIMEOUT).unwrap_or_else(|_| Report {
        error: Some("the server thread never reported".to_string()),
        ..Report::default()
    });

    let status = child.wait().expect("wait");
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    let stdout = reader.join().unwrap_or_default();

    assert!(
        echoed || report.error.is_some(),
        "the probe never came back and the server reported no error; \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );

    Session {
        report,
        stdout,
        stderr,
        status,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The headline: an independent TLS 1.3 client completes a handshake with this
/// server over a socket, and verifies the chain while doing it.
#[test]
#[ignore = "needs the `openssl` binary; see the module docs"]
fn openssl_completes_a_handshake_and_verifies_the_chain() {
    let session = session("hello from openssl\n", "handshake");

    assert!(
        session.report.error.is_none(),
        "server: {:?}\nstdout:\n{}\nstderr:\n{}",
        session.report.error,
        session.stdout,
        session.stderr,
    );
    assert!(
        session.report.handshake_completed,
        "the handshake did not complete\nstderr:\n{}",
        session.stderr,
    );

    // `-verify_return_error` turns a bad chain into a non-zero exit, so this
    // is the assertion that the certificate was trusted rather than merely
    // parsed.
    assert!(
        session.status.success(),
        "openssl exited {:?}, so it did not accept the chain\nstdout:\n{}\nstderr:\n{}",
        session.status.code(),
        session.stdout,
        session.stderr,
    );
    assert!(
        session.stdout.contains("Verify return code: 0 (ok)"),
        "openssl did not report a clean verification\nstdout:\n{}",
        session.stdout,
    );
}

/// Application data crosses in both directions after the handshake.
///
/// The echo is observed from *OpenSSL's* stdout, not only from the server's
/// own bookkeeping — a server that thought it had replied but sealed something
/// the peer could not open would pass the weaker check.
#[test]
#[ignore = "needs the `openssl` binary; see the module docs"]
fn application_data_round_trips_through_openssl() {
    let probe = "the quick brown fox jumps over the lazy dog\n";
    let session = session(probe, "roundtrip");

    assert!(
        session.report.error.is_none(),
        "server: {:?}",
        session.report.error
    );
    assert_eq!(
        session.report.echoed.as_deref(),
        Some(probe.as_bytes()),
        "the server did not see the probe it should have echoed",
    );
    assert!(
        session.stdout.contains(probe.trim()),
        "the echo never reached openssl\nstdout:\n{}",
        session.stdout,
    );
}

/// A `close_notify` from an independent client is an orderly close, not an
/// error.
///
/// Worth its own test because the failure is silent in the wrong direction: a
/// server that reported every completed exchange as a broken connection would
/// still pass every handshake test above it.
#[test]
#[ignore = "needs the `openssl` binary; see the module docs"]
fn a_close_from_openssl_is_seen_as_a_close() {
    let session = session("closing\n", "close");

    assert!(
        session.report.error.is_none(),
        "server: {:?}",
        session.report.error
    );
    assert!(
        session.report.closed_cleanly,
        "the server never saw close_notify; it reported {:?}",
        session.report,
    );
}

/// What OpenSSL actually negotiated, and how many records it took.
///
/// Reported rather than asserted, for the reason the client-side interop suite
/// gives: the number of records another implementation chooses to send is a
/// fact about that implementation, and pinning it here would make this suite
/// fail when OpenSSL changes something it is entitled to change.
#[test]
#[ignore = "needs the `openssl` binary; see the module docs"]
fn what_openssl_negotiated_is_reported() {
    let session = session("report\n", "report");

    assert!(
        session.report.error.is_none(),
        "server: {:?}",
        session.report.error
    );

    for line in session.stdout.lines() {
        let line = line.trim();
        // "New, TLSv1.3, Cipher is ..." is the line that actually names the
        // suite; the `SSL-Session` block's `Protocol`/`Cipher` fields are not
        // always flushed before the connection ends.
        if line.starts_with("New,")
            || line.starts_with("Protocol")
            || line.starts_with("Cipher")
            || line.starts_with("Server Temp Key")
            || line.starts_with("Peer signature type")
            || line.starts_with("Verify return code")
        {
            println!("openssl: {line}");
        }
    }
    println!(
        "handshake records received from openssl: {}",
        session.report.handshake_records
    );

    // The one thing worth asserting: it must be 1.3, because `-tls1_3` was
    // passed and anything else means the flag did not do what it says.
    assert!(
        session.stdout.contains("TLSv1.3"),
        "openssl did not report TLS 1.3\nstdout:\n{}",
        session.stdout,
    );
}
