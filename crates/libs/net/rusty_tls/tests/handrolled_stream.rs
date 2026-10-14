//! `NativeTlsStream` as a consumer would use it: over a real socket, against a
//! real rustls server, through `Read` and `Write`.
//!
//! The engine's own suites drive its state machines record by record. This one
//! checks the thing built on top of them: lazy handshake, buffering across
//! record boundaries, fragmentation of a large write, and, above all, what a
//! caller sees when the connection ends, whether cleanly or not.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::stream::NativeTlsStream;
use rusty_tls::{Error, TrustPolicy};

const NAME: &str = "stream.example";

struct Pki {
    root: CertificateDer<'static>,
    leaf: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

fn pki(name: &str) -> Pki {
    let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("root params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let root = root_params.self_signed(&root_key).expect("root");

    let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("leaf key");
    let leaf_params = CertificateParams::new(vec![name.to_string()]).expect("leaf params");
    let leaf = leaf_params
        .signed_by(&leaf_key, &root, &root_key)
        .expect("leaf");
    Pki {
        root: CertificateDer::from(root.der().to_vec()),
        leaf: CertificateDer::from(leaf.der().to_vec()),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    }
}

/// How the server ends the conversation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// Echo until the client is done, then close properly.
    EchoUntilClose,
    /// Send a greeting, then drop the TCP connection with no `close_notify`.
    GreetThenDrop,
    /// Send a greeting, then send `close_notify`.
    GreetThenCloseNotify,
    /// Ask the client to roll keys, say "ping", wait for "pong", say "done".
    KeyUpdateThenPingPong,
}

/// What the server saw of the client's end.
#[derive(Debug, Default)]
struct Seen {
    /// The client's `close_notify` arrived as an orderly end of stream.
    clean_close: bool,
}

fn server_config(
    pki: &Pki,
    versions: &[&'static rustls::SupportedProtocolVersion],
    alpn: &[&[u8]],
) -> Arc<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::builder_with_protocol_versions(versions)
        .with_no_client_auth()
        .with_single_cert(vec![pki.leaf.clone()], pki.key.clone_key())
        .expect("server config");
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Arc::new(config)
}

/// Serve one connection on a loopback port.
fn serve(config: Arc<rustls::ServerConfig>, ending: Ending) -> (u16, JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = thread::spawn(move || {
        let (tcp, _) = listener.accept().expect("accept");
        tcp.set_read_timeout(Some(Duration::from_secs(20))).ok();
        let conn = rustls::ServerConnection::new(config).expect("server connection");
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        let mut seen = Seen::default();
        match ending {
            Ending::EchoUntilClose => {
                let mut buf = vec![0u8; 8192];
                loop {
                    match tls.read(&mut buf) {
                        Ok(0) => {
                            seen.clean_close = true;
                            break;
                        }
                        Ok(n) => {
                            if tls.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
            Ending::GreetThenDrop => {
                let _ = tls.write_all(b"bye");
                let _ = tls.flush();
                // Dropping `tls` closes the socket with no close_notify.
            }
            Ending::GreetThenCloseNotify => {
                let _ = tls.write_all(b"bye");
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
            Ending::KeyUpdateThenPingPong => {
                let mut pong = [0u8; 4];
                let ok = tls.conn.complete_io(&mut tls.sock).is_ok()
                    && tls.conn.refresh_traffic_keys().is_ok()
                    && tls.write_all(b"ping").is_ok()
                    && tls.flush().is_ok()
                    && tls.read_exact(&mut pong).is_ok()
                    && &pong == b"pong"
                    && tls.write_all(b"done").is_ok()
                    && tls.flush().is_ok();
                seen.clean_close = ok;
            }
        }
        seen
    });
    (port, handle)
}

fn connect(port: u16) -> TcpStream {
    let tcp = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    tcp.set_read_timeout(Some(Duration::from_secs(20))).ok();
    tcp
}

fn trust(pki: &Pki) -> TrustPolicy {
    TrustPolicy::PinnedAnchors(vec![pki.root.clone()])
}

fn read_exactly<R: Read>(stream: &mut R, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    stream.read_exact(&mut out).expect("reads the echo");
    out
}

// ---------------------------------------------------------------------------

#[test]
fn it_echoes_on_each_tls_version_and_reports_what_was_negotiated() {
    for (label, versions) in [
        ("1.3", &[&rustls::version::TLS13][..]),
        ("1.2", &[&rustls::version::TLS12][..]),
    ] {
        let pki = pki(NAME);
        let (port, server) = serve(
            server_config(&pki, versions, &[b"h2", b"http/1.1"]),
            Ending::EchoUntilClose,
        );
        let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki))
            .expect("builds")
            .with_alpn(vec![b"http/1.1".to_vec()])
            .expect("a valid offer");
        assert!(stream.is_handshaking(), "{label}: no I/O at construction");

        stream.complete_handshake().expect("handshake");
        assert!(!stream.is_handshaking());
        assert_eq!(
            stream.peer_certificate_der(),
            Some(pki.leaf.as_ref()),
            "{label}: the server's leaf"
        );
        assert_eq!(
            stream.negotiated_alpn_protocol(),
            Some(&b"http/1.1"[..]),
            "{label}: ALPN"
        );

        stream.write_all(b"hello").expect("writes");
        assert_eq!(read_exactly(&mut stream, 5), b"hello", "{label}");

        stream.shutdown().expect("close_notify");
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).expect("the peer's close");
        assert!(rest.is_empty());
        assert!(
            server.join().expect("server").clean_close,
            "{label}: the server saw an orderly close"
        );
    }
}

#[test]
fn the_handshake_runs_on_first_use_when_not_asked_for() {
    let pki = pki(NAME);
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS13], &[]),
        Ending::EchoUntilClose,
    );
    let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki)).expect("builds");
    stream
        .write_all(b"first")
        .expect("writes, handshaking first");
    assert_eq!(read_exactly(&mut stream, 5), b"first");
    drop(stream);
    let _ = server.join();
}

#[test]
fn a_write_larger_than_a_record_survives_fragmentation_and_buffering() {
    for versions in [
        &[&rustls::version::TLS13][..],
        &[&rustls::version::TLS12][..],
    ] {
        let pki = pki(NAME);
        let (port, server) = serve(server_config(&pki, versions, &[]), Ending::EchoUntilClose);
        let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki)).expect("builds");

        let payload: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        stream.write_all(&payload).expect("writes 100 KB");
        // Read it back in small, uneven pieces, so buffering across record
        // boundaries is exercised.
        let mut got = Vec::new();
        let mut piece = 1usize;
        while got.len() < payload.len() {
            let want = piece.min(payload.len() - got.len());
            got.extend(read_exactly(&mut stream, want));
            piece = piece * 3 % 4099 + 1;
        }
        assert_eq!(got, payload);
        drop(stream);
        let _ = server.join();
    }
}

#[test]
fn a_server_the_policy_does_not_trust_is_refused_and_nothing_is_delivered() {
    let server_pki = pki(NAME);
    let other = pki(NAME);
    let (port, server) = serve(
        server_config(&server_pki, &[&rustls::version::TLS13], &[]),
        Ending::GreetThenCloseNotify,
    );
    let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&other)).expect("builds");
    let err = stream.complete_handshake().expect_err("untrusted");
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");

    // The stream stays failed: no read or write gets through afterwards.
    let mut buf = [0u8; 8];
    assert!(stream.read(&mut buf).is_err());
    assert!(stream.write(b"x").is_err());
    // Close our end so the server's handshake read ends instead of timing out.
    drop(stream);
    let _ = server.join();
}

#[test]
fn a_certificate_for_another_name_is_refused() {
    let pki = pki("someone-else.example");
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS13], &[]),
        Ending::GreetThenCloseNotify,
    );
    let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki)).expect("builds");
    let err = stream.complete_handshake().expect_err("wrong name");
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");
    drop(stream);
    let _ = server.join();
}

#[test]
fn a_connection_dropped_without_close_notify_is_a_truncation_not_an_end() {
    for versions in [
        &[&rustls::version::TLS13][..],
        &[&rustls::version::TLS12][..],
    ] {
        let pki = pki(NAME);
        let (port, server) = serve(server_config(&pki, versions, &[]), Ending::GreetThenDrop);
        let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki)).expect("builds");
        assert_eq!(read_exactly(&mut stream, 3), b"bye");
        let mut buf = [0u8; 8];
        let err = stream.read(&mut buf).expect_err("no close_notify");
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof, "{err}");
        let _ = server.join();
    }
}

#[test]
fn an_orderly_close_ends_the_stream_and_stays_ended() {
    for versions in [
        &[&rustls::version::TLS13][..],
        &[&rustls::version::TLS12][..],
    ] {
        let pki = pki(NAME);
        let (port, server) = serve(
            server_config(&pki, versions, &[]),
            Ending::GreetThenCloseNotify,
        );
        let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki)).expect("builds");
        assert_eq!(read_exactly(&mut stream, 3), b"bye");
        let mut buf = [0u8; 8];
        assert_eq!(stream.read(&mut buf).expect("close_notify"), 0);
        assert_eq!(stream.read(&mut buf).expect("still ended"), 0);
        // No writing into a connection the peer has closed.
        let err = stream.write(b"late").expect_err("closed");
        assert_eq!(err.kind(), ErrorKind::BrokenPipe);
        let _ = server.join();
    }
}

#[test]
fn trust_policies_the_engine_cannot_honour_are_refused_up_front() {
    let sock = || TcpStream::connect("127.0.0.1:9").ok();
    // A listener that is never connected to, just so there is a socket to wrap.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let tcp = TcpStream::connect(listener.local_addr().expect("addr")).expect("connect");
    let _ = sock;

    let err = NativeTlsStream::new(tcp, NAME, &TrustPolicy::DangerNoVerification)
        .expect_err("no verification");
    match err {
        Error::Io(e) => assert_eq!(e.kind(), ErrorKind::Unsupported),
        other => panic!("expected an Unsupported I/O error, got {other}"),
    }

    let tcp = TcpStream::connect(listener.local_addr().expect("addr")).expect("connect");
    assert!(matches!(
        NativeTlsStream::new(tcp, NAME, &TrustPolicy::PinnedAnchors(Vec::new())),
        Err(Error::NoTrustAnchors)
    ));

    let tcp = TcpStream::connect(listener.local_addr().expect("addr")).expect("connect");
    assert!(matches!(
        NativeTlsStream::new(tcp, "", &TrustPolicy::System),
        Err(Error::InvalidServerName(_))
    ));
}

#[test]
fn the_debug_output_names_no_key_material() {
    let pki = pki(NAME);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let tcp = TcpStream::connect(listener.local_addr().expect("addr")).expect("connect");
    let stream = NativeTlsStream::new(tcp, NAME, &trust(&pki)).expect("builds");
    let printed = format!("{stream:?}");
    assert!(printed.starts_with("NativeTlsStream"), "{printed}");
    assert!(
        !printed.contains("secret") && !printed.contains("key"),
        "{printed}"
    );
}

// ---------------------------------------------------------------------------
// A transport that buffers like a `BufWriter` and fails on demand
// ---------------------------------------------------------------------------

/// What the stream did to its transport.
#[derive(Default)]
struct Wire {
    /// Reads started while written bytes were still unflushed. A client that
    /// does this waits for an answer to something it never sent.
    unflushed_reads: AtomicUsize,
    /// Calls made after the transport had already returned an error.
    calls_after_failure: AtomicUsize,
    /// Bytes still allowed through before writes fail; negative means no limit.
    budget: AtomicIsize,
    /// `flush` fails while set.
    fail_flush: std::sync::atomic::AtomicBool,
    /// Bytes handed to `write`, whether or not they were flushed on.
    written: AtomicUsize,
    failed: std::sync::atomic::AtomicBool,
}

impl Wire {
    fn new() -> Arc<Self> {
        let wire = Self::default();
        wire.budget.store(-1, Ordering::SeqCst);
        Arc::new(wire)
    }

    fn arm_write_failure_after(&self, bytes: isize) {
        self.budget.store(bytes, Ordering::SeqCst);
    }

    fn note_call(&self) {
        if self.failed.load(Ordering::SeqCst) {
            self.calls_after_failure.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn fail(&self) -> io::Error {
        self.failed.store(true, Ordering::SeqCst);
        io::Error::new(ErrorKind::BrokenPipe, "injected transport failure")
    }
}

/// Buffers every write until `flush`, like `BufWriter<TcpStream>`.
struct BufferedWire {
    inner: TcpStream,
    buf: Vec<u8>,
    wire: Arc<Wire>,
}

impl BufferedWire {
    fn new(inner: TcpStream, wire: &Arc<Wire>) -> Self {
        Self {
            inner,
            buf: Vec::new(),
            wire: Arc::clone(wire),
        }
    }
}

impl Read for BufferedWire {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.wire.note_call();
        if !self.buf.is_empty() {
            self.wire.unflushed_reads.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.read(out)
    }
}

impl Write for BufferedWire {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.wire.note_call();
        let budget = self.wire.budget.load(Ordering::SeqCst);
        let n = if budget < 0 {
            data.len()
        } else if budget == 0 {
            return Err(self.wire.fail());
        } else {
            data.len().min(budget as usize)
        };
        if budget >= 0 {
            self.wire.budget.fetch_sub(n as isize, Ordering::SeqCst);
        }
        self.wire.written.fetch_add(n, Ordering::SeqCst);
        self.buf.extend_from_slice(&data[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.wire.note_call();
        if self.wire.fail_flush.load(Ordering::SeqCst) {
            return Err(self.wire.fail());
        }
        self.inner.write_all(&self.buf)?;
        self.buf.clear();
        self.inner.flush()
    }
}

fn buffered(port: u16, pki: &Pki, wire: &Arc<Wire>) -> NativeTlsStream<BufferedWire> {
    let tcp = connect(port);
    tcp.set_read_timeout(Some(Duration::from_secs(5))).ok();
    NativeTlsStream::new(BufferedWire::new(tcp, wire), NAME, &trust(pki)).expect("builds")
}

// ---------------------------------------------------------------------------
// Flushing: a buffered transport must not hold a flight we are waiting on
// ---------------------------------------------------------------------------

fn retry_provider() -> Arc<rustls::crypto::CryptoProvider> {
    let mut provider = rustls::crypto::ring::default_provider();
    // Only P-256: the client's first key share (X25519) is wrong, so the server
    // answers with a HelloRetryRequest and the client sends a second flight.
    provider.kx_groups = vec![rustls::crypto::ring::kx_group::SECP256R1];
    Arc::new(provider)
}

#[test]
fn handshake_flights_are_flushed_before_the_stream_waits_on_a_buffered_transport() {
    for label in ["1.3", "1.2", "1.3 with a HelloRetryRequest"] {
        let pki = pki(NAME);
        let config = match label {
            "1.3" => server_config(&pki, &[&rustls::version::TLS13], &[]),
            "1.2" => server_config(&pki, &[&rustls::version::TLS12], &[]),
            _ => Arc::new(
                rustls::ServerConfig::builder_with_provider(retry_provider())
                    .with_protocol_versions(&[&rustls::version::TLS13])
                    .expect("versions")
                    .with_no_client_auth()
                    .with_single_cert(vec![pki.leaf.clone()], pki.key.clone_key())
                    .expect("config"),
            ),
        };
        let (port, server) = serve(config, Ending::EchoUntilClose);
        let wire = Wire::new();
        let mut stream = buffered(port, &pki, &wire);

        stream
            .complete_handshake()
            .unwrap_or_else(|e| panic!("{label}: the handshake stalled on buffered writes: {e}"));
        stream.write_all(b"hello").expect("writes");
        stream.flush().expect("flushes");
        assert_eq!(read_exactly(&mut stream, 5), b"hello", "{label}");
        assert_eq!(
            wire.unflushed_reads.load(Ordering::SeqCst),
            0,
            "{label}: the stream blocked on a read with its own bytes still buffered"
        );
        drop(stream);
        let _ = server.join();
    }
}

#[test]
fn a_requested_key_update_is_answered_and_flushed_before_the_stream_waits() {
    let pki = pki(NAME);
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS13], &[]),
        Ending::KeyUpdateThenPingPong,
    );
    let wire = Wire::new();
    let mut stream = buffered(port, &pki, &wire);

    assert_eq!(read_exactly(&mut stream, 4), b"ping");
    stream
        .write_all(b"pong")
        .expect("writes under the new keys");
    stream.flush().expect("flushes");
    assert_eq!(read_exactly(&mut stream, 4), b"done");
    assert_eq!(
        wire.unflushed_reads.load(Ordering::SeqCst),
        0,
        "the KeyUpdate answer sat in the buffer while the stream waited"
    );
    drop(stream);
    assert!(
        server.join().expect("server").clean_close,
        "server saw the exchange"
    );
}

// ---------------------------------------------------------------------------
// Failure is permanent, and nothing pretends otherwise
// ---------------------------------------------------------------------------

fn established_over_buffered_wire() -> (NativeTlsStream<BufferedWire>, Arc<Wire>, JoinHandle<Seen>)
{
    let pki = pki(NAME);
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS13], &[]),
        Ending::EchoUntilClose,
    );
    let wire = Wire::new();
    let mut stream = buffered(port, &pki, &wire);
    stream.complete_handshake().expect("handshake");
    (stream, wire, server)
}

#[test]
fn a_failed_data_write_fails_the_stream_for_good_and_later_calls_do_no_io() {
    let (mut stream, wire, server) = established_over_buffered_wire();
    // Let part of the sealed record through, then fail: the record layer has
    // already moved its sequence number, so the rest cannot be re-sent.
    wire.arm_write_failure_after(10);
    assert!(stream.write_all(&[7u8; 100]).is_err());

    let before = wire.calls_after_failure.load(Ordering::SeqCst);
    let mut buf = [0u8; 8];
    assert!(stream.write(b"x").is_err(), "later writes fail");
    assert!(stream.flush().is_err(), "flush fails");
    assert!(stream.read(&mut buf).is_err(), "reads fail");
    assert!(
        stream.shutdown().is_err(),
        "shutdown does not claim success"
    );
    assert_eq!(
        wire.calls_after_failure.load(Ordering::SeqCst),
        before,
        "the stream touched a transport that had failed"
    );
    drop(stream);
    let _ = server.join();
}

#[test]
fn a_failed_flush_fails_the_stream_for_good() {
    let (mut stream, wire, server) = established_over_buffered_wire();
    stream.write_all(b"data").expect("buffered");
    wire.fail_flush.store(true, Ordering::SeqCst);
    assert!(stream.flush().is_err());

    let before = wire.calls_after_failure.load(Ordering::SeqCst);
    assert!(stream.write(b"more").is_err());
    assert!(stream.flush().is_err());
    assert_eq!(wire.calls_after_failure.load(Ordering::SeqCst), before);
    drop(stream);
    let _ = server.join();
}

#[test]
fn a_close_notify_that_cannot_be_delivered_is_never_reported_as_sent() {
    let (mut stream, wire, server) = established_over_buffered_wire();
    wire.arm_write_failure_after(0);
    assert!(stream.shutdown().is_err(), "the close did not go out");

    let before = wire.calls_after_failure.load(Ordering::SeqCst);
    assert!(
        stream.shutdown().is_err(),
        "a retry must not turn a lost close_notify into success"
    );
    assert!(stream.write(b"x").is_err());
    assert_eq!(wire.calls_after_failure.load(Ordering::SeqCst), before);
    drop(stream);
    let _ = server.join();
}

#[test]
fn a_shutdown_that_was_delivered_stays_ok_when_repeated() {
    let (mut stream, _wire, server) = established_over_buffered_wire();
    stream.shutdown().expect("delivered");
    stream.shutdown().expect("idempotent");
    drop(stream);
    let _ = server.join();
}

#[test]
fn a_control_reply_that_cannot_be_sent_fails_the_stream() {
    let pki = pki(NAME);
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS13], &[]),
        Ending::KeyUpdateThenPingPong,
    );
    let wire = Wire::new();
    let mut stream = buffered(port, &pki, &wire);
    stream.complete_handshake().expect("handshake");

    // The server's KeyUpdate asks for an answer; make sending it fail.
    wire.arm_write_failure_after(0);
    let mut buf = [0u8; 8];
    assert!(
        stream.read(&mut buf).is_err(),
        "the reply could not be sent"
    );

    let before = wire.calls_after_failure.load(Ordering::SeqCst);
    assert!(stream.read(&mut buf).is_err());
    assert!(stream.write(b"x").is_err());
    assert_eq!(wire.calls_after_failure.load(Ordering::SeqCst), before);
    drop(stream);
    let _ = server.join();
}

// ---------------------------------------------------------------------------
// ALPN offers are checked before any I/O
// ---------------------------------------------------------------------------

fn alpn_result(protocols: Vec<Vec<u8>>) -> (io::Result<()>, usize) {
    let pki = pki(NAME);
    let wire = Wire::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let tcp = TcpStream::connect(listener.local_addr().expect("addr")).expect("connect");
    let stream = NativeTlsStream::new(BufferedWire::new(tcp, &wire), NAME, &trust(&pki))
        .expect("builds")
        .with_alpn(protocols);
    (stream.map(|_| ()), wire.written.load(Ordering::SeqCst))
}

#[test]
fn alpn_names_and_offers_outside_the_limits_are_refused_before_any_io() {
    let name = |n: usize| vec![b'a'; n];
    for (label, offer) in [
        ("an empty name", vec![name(0)]),
        ("a 256-byte name", vec![name(256)]),
        ("a 1000-byte name", vec![name(1000)]),
        (
            "an empty name among good ones",
            vec![name(2), name(0), name(3)],
        ),
        // 16 names of 255 bytes frame to exactly 4096; one more byte tips it over.
        ("an offer one byte over the aggregate limit", {
            let mut v = vec![name(255); 15];
            v.push(name(254));
            v.push(name(2));
            v
        }),
        ("a huge offer", vec![name(255); 400]),
    ] {
        let (result, written) = alpn_result(offer);
        let err = result.expect_err(label);
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{label}: {err}");
        assert_eq!(written, 0, "{label}: no I/O before the offer is checked");
    }
}

#[test]
fn alpn_offers_at_the_limits_are_accepted() {
    let name = |n: usize| vec![b'a'; n];
    for (label, offer) in [
        ("no offer", Vec::new()),
        ("a 1-byte name", vec![name(1)]),
        ("a 255-byte name", vec![name(255)]),
        (
            "h2 and http/1.1",
            vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        ),
        ("exactly the aggregate limit", vec![name(255); 16]),
    ] {
        let (result, _) = alpn_result(offer);
        result.unwrap_or_else(|e| panic!("{label}: {e}"));
    }
}

// ---------------------------------------------------------------------------
// `tls13_only`: the enforced form of "TLS 1.3 only".

/// Read the ClientHello `stream` sends first, then drop the connection.
fn first_hello(build: impl FnOnce(TcpStream) -> NativeTlsStream<TcpStream>) -> Vec<u8> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let server = thread::spawn(move || {
        let (mut tcp, _) = listener.accept().expect("accept");
        tcp.set_read_timeout(Some(Duration::from_secs(20))).ok();
        let mut header = [0u8; 5];
        tcp.read_exact(&mut header).expect("record header");
        let mut body = vec![0u8; usize::from(u16::from_be_bytes([header[3], header[4]]))];
        tcp.read_exact(&mut body).expect("record body");
        body
    });
    let mut stream = build(connect(port));
    let _ = stream.complete_handshake(); // the "server" never answers
    drop(stream);
    server.join().expect("server")
}

/// The cipher suites and the `supported_versions` list of a ClientHello body.
fn hello_offer(hello: &[u8]) -> (Vec<u16>, Vec<u16>) {
    let u16_at = |at: usize| usize::from(u16::from_be_bytes([hello[at], hello[at + 1]]));
    assert_eq!(hello[0], 1, "a ClientHello");
    let mut at = 4 + 2 + 32; // handshake header, legacy_version, random
    at += 1 + usize::from(hello[at]); // legacy_session_id
    let suites_len = u16_at(at);
    let suites: Vec<u16> = hello[at + 2..at + 2 + suites_len]
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    at += 2 + suites_len;
    at += 1 + usize::from(hello[at]); // legacy_compression_methods
    let (mut ext, end) = (at + 2, at + 2 + u16_at(at));
    let mut versions = Vec::new();
    while ext < end {
        let (kind, len) = (u16_at(ext), u16_at(ext + 2));
        if kind == 0x002b {
            versions = hello[ext + 5..ext + 4 + len]
                .chunks(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
        }
        ext += 4 + len;
    }
    (suites, versions)
}

#[test]
fn a_tls13_only_hello_offers_no_tls12_version_suite_or_fallback_signal() {
    let pki = pki(NAME);
    let both = first_hello(|tcp| NativeTlsStream::new(tcp, NAME, &trust(&pki)).expect("builds"));
    let only = first_hello(|tcp| {
        NativeTlsStream::new(tcp, NAME, &trust(&pki))
            .expect("builds")
            .tls13_only()
            .expect("before the handshake")
    });

    // The parse sees what a dual-version hello offers, so the next asserts mean something.
    let (suites, versions) = hello_offer(&both);
    assert!(
        versions.contains(&0x0303),
        "the default offers 1.2: {versions:x?}"
    );
    assert!(CipherSuite12::SUPPORTED
        .iter()
        .all(|s| suites.contains(&s.0)));

    let (suites, versions) = hello_offer(&only);
    assert_eq!(versions, [0x0304], "supported_versions is exactly TLS 1.3");
    for suite in CipherSuite12::SUPPORTED {
        assert!(
            !suites.contains(&suite.0),
            "no TLS 1.2 suite {:#06x}",
            suite.0
        );
    }
    assert!(!suites.contains(&0x5600), "no TLS_FALLBACK_SCSV");
}

#[test]
fn tls13_only_completes_against_a_tls13_server() {
    let pki = pki(NAME);
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS13], &[]),
        Ending::EchoUntilClose,
    );
    let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki))
        .expect("builds")
        .tls13_only()
        .expect("before the handshake");
    stream.write_all(b"hello").expect("writes");
    assert_eq!(read_exactly(&mut stream, 5), b"hello");
    stream.shutdown().expect("close_notify");
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).expect("the peer's close");
    assert!(server.join().expect("server").clean_close);
}

#[test]
fn tls13_only_refuses_a_tls12_only_server_and_stays_failed() {
    let pki = pki(NAME);
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS12], &[]),
        Ending::GreetThenCloseNotify,
    );
    let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki))
        .expect("builds")
        .tls13_only()
        .expect("before the handshake");
    let err = stream.complete_handshake().expect_err("no TLS 1.3");
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");
    assert!(
        stream.peer_certificate_der().is_none(),
        "nothing was established"
    );

    let mut buf = [0u8; 8];
    assert!(stream.read(&mut buf).is_err());
    assert!(stream.write(b"x").is_err());
    drop(stream);
    let _ = server.join();
}

#[test]
fn tls13_only_after_the_handshake_is_refused_and_does_not_pretend() {
    let pki = pki(NAME);
    let (port, server) = serve(
        server_config(&pki, &[&rustls::version::TLS12], &[]),
        Ending::EchoUntilClose,
    );
    let mut stream = NativeTlsStream::new(connect(port), NAME, &trust(&pki)).expect("builds");
    stream.complete_handshake().expect("TLS 1.2 handshake");
    let err = stream.tls13_only().expect_err("too late");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let _ = server.join();
}

/// A transport that serves one script of records and counts how many were read.
struct Script {
    records: Vec<Vec<u8>>,
    next: usize,
    pending: Vec<u8>,
}

impl Read for Script {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            let Some(record) = self.records.get(self.next) else {
                return Err(io::Error::new(ErrorKind::UnexpectedEof, "script ended"));
            };
            self.pending = record.clone();
            self.next += 1;
        }
        let n = out.len().min(self.pending.len());
        out[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

impl Write for Script {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn tls13_only_bounds_handshake_reassembly_before_the_server_hello() {
    // A handshake message that declares ~16 MiB, delivered in legal 16 KiB
    // records: 02 ff ff ff, then zeros. Nothing here is a valid ServerHello.
    const RECORD: usize = 1 << 14;
    const TOTAL: usize = 100; // 1.6 MiB on offer
    let records: Vec<Vec<u8>> = (0..TOTAL)
        .map(|i| {
            let mut body = vec![0u8; RECORD];
            if i == 0 {
                body[..4].copy_from_slice(&[2, 0xff, 0xff, 0xff]);
            }
            let mut record = vec![22, 3, 3];
            record.extend_from_slice(&(RECORD as u16).to_be_bytes());
            record.extend_from_slice(&body);
            record
        })
        .collect();
    let script = Script {
        records,
        next: 0,
        pending: Vec::new(),
    };
    let mut stream = NativeTlsStream::new(script, NAME, &trust(&pki(NAME)))
        .expect("builds")
        .tls13_only()
        .expect("before the handshake");

    let err = stream.complete_handshake().expect_err("oversized");
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");
    // 128 KiB is eight records; the ninth crosses the bound. The rest are never read.
    assert!(
        stream.get_ref().next <= 9,
        "read {} of {TOTAL} records before refusing",
        stream.get_ref().next
    );
    let after = stream.get_ref().next;
    assert!(
        stream.complete_handshake().is_err(),
        "the failure is permanent"
    );
    assert_eq!(stream.get_ref().next, after, "and reads nothing more");
}
