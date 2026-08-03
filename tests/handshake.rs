//! Hermetic handshake tests, against a local rustls test server — no
//! network access, no real CA.
//!
//! The design record behind this crate (rustils' `design-discussion-tls.md`)
//! makes the point this suite exists to honor: TLS failures are silent by
//! default — a validator that accepts a bad chain passes every happy-path
//! test ever written for it. So the rejection tests below outnumber the
//! success test on purpose; they are the point, not an afterthought.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use rusty_tls::{TlsStream, TrustPolicy};

/// The rejection cases, shared with the hand-rolled engine's suite. This file
/// holds the `rustls` driver for them; `handrolled_client.rs` holds the other.
mod rejection;

struct TestCa {
    cert: Certificate,
    key_pair: KeyPair,
}

impl TestCa {
    fn generate() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "rusty_tls test CA");
        params.distinguished_name = dn;
        let key_pair = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        TestCa { cert, key_pair }
    }

    fn root_der(&self) -> CertificateDer<'static> {
        self.cert.der().clone()
    }

    /// Issue a leaf certificate valid for `hostname`, with the given
    /// validity window (defaults to a long window if `None`).
    fn issue_leaf(
        &self,
        hostname: &str,
        validity: Option<(time::OffsetDateTime, time::OffsetDateTime)>,
    ) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let mut params = CertificateParams::new(vec![hostname.to_string()]).unwrap();
        if let Some((not_before, not_after)) = validity {
            params.not_before = not_before;
            params.not_after = not_after;
        }
        let leaf_key = KeyPair::generate().unwrap();
        let leaf_cert = params
            .signed_by(&leaf_key, &self.cert, &self.key_pair)
            .unwrap();
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        (leaf_cert.der().clone(), key_der)
    }
}

fn self_signed_leaf(hostname: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
    (cert.der().clone(), key_der)
}

/// Spin up a one-shot TLS echo server on a background thread: accept one
/// connection, complete the handshake, echo back whatever the client sends.
/// Errors (including a client that never completes the handshake — expected
/// for the rejection tests below) are swallowed: the client-side assertion
/// is always what these tests check.
fn spawn_echo_server(
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    spawn_echo_server_with_chain(vec![cert_der], key_der)
}

/// As [`spawn_echo_server`], but presenting a full chain rather than a single
/// certificate — what the shared rejection table builds.
fn spawn_echo_server_with_chain(
    chain: Vec<CertificateDer<'static>>,
    key_der: PrivateKeyDer<'static>,
) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(chain, key_der)
            .expect("valid test cert/key"),
    );

    let handle = thread::spawn(move || {
        let Ok((tcp, _)) = listener.accept() else {
            return;
        };
        let Ok(conn) = ServerConnection::new(config) else {
            return;
        };
        let mut tls = StreamOwned::new(conn, tcp);
        let mut buf = [0u8; 1024];
        if let Ok(n) = tls.read(&mut buf) {
            let _ = tls.write_all(&buf[..n]);
        }
    });

    (addr, handle)
}

#[test]
fn handshake_succeeds_and_round_trips_with_pinned_anchor() {
    let ca = TestCa::generate();
    let (leaf_der, leaf_key) = ca.issue_leaf("localhost", None);
    let (addr, server) = spawn_echo_server(leaf_der, leaf_key);

    let tcp = TcpStream::connect(addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![ca.root_der()]);
    let mut tls = TlsStream::new(tcp, "localhost", &policy).unwrap();

    tls.write_all(b"hello, rusty_tls").unwrap();
    let mut buf = [0u8; "hello, rusty_tls".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"hello, rusty_tls");

    server.join().unwrap();
}

#[test]
fn danger_no_verification_accepts_a_self_signed_certificate() {
    let (leaf_der, leaf_key) = self_signed_leaf("localhost");
    let (addr, server) = spawn_echo_server(leaf_der, leaf_key);

    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();

    tls.write_all(b"trust me").unwrap();
    let mut buf = [0u8; "trust me".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"trust me");

    server.join().unwrap();
}

/// The `rustls` driver for the shared rejection table.
///
/// This replaces three hand-written rejection tests that had hand-written
/// counterparts in `handrolled_client.rs`. The cases now live in
/// `tests/rejection/mod.rs` and both engines run the same rows against the
/// same certificates — see that module for why the two suites agreeing was a
/// weaker claim than one table run twice.
///
/// Every case is attempted before anything is asserted. A `panic!` on the
/// first mismatch would report one row and hide the rest, which is the wrong
/// shape for a parity table: what you want to know is *which* rows moved.
#[test]
fn the_shared_rejection_table_holds_for_rustls() {
    rejection::assert_table_is_coherent();

    let mut failures = Vec::new();

    for case in rejection::CASES {
        let fixture = rejection::fixture(case);
        let (addr, server) =
            spawn_echo_server_with_chain(fixture.chain.clone(), fixture.key.clone_key());

        let tcp = TcpStream::connect(addr).unwrap();
        let policy = TrustPolicy::PinnedAnchors(vec![CertificateDer::from(
            fixture.trusted_root_der.clone(),
        )]);

        // `TlsStream::new` does no I/O — the handshake runs on first use, so
        // the verdict is whatever the first write/read says.
        let observed = match TlsStream::new(tcp, case.requested_name, &policy) {
            Err(_) => rejection::Outcome::Rejected,
            Ok(mut tls) => match tls.write_all(b"probe").and_then(|()| tls.flush()) {
                Err(_) => rejection::Outcome::Rejected,
                Ok(()) => {
                    let mut buf = [0u8; b"probe".len()];
                    match tls.read_exact(&mut buf) {
                        Ok(()) if buf == *b"probe" => rejection::Outcome::Accepted,
                        _ => rejection::Outcome::Rejected,
                    }
                }
            },
        };

        let _ = server.join();

        if observed != case.rustls {
            failures.push(format!(
                "{}: expected rustls to have {:?} it, observed {:?}",
                case.name, case.rustls, observed
            ));
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn complete_handshake_exposes_the_peers_certificate() {
    let ca = TestCa::generate();
    let (leaf_der, leaf_key) = ca.issue_leaf("localhost", None);
    let (addr, server) = spawn_echo_server(leaf_der.clone(), leaf_key);

    let tcp = TcpStream::connect(addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![ca.root_der()]);
    let mut tls = TlsStream::new(tcp, "localhost", &policy).unwrap();

    assert!(tls.is_handshaking());
    assert_eq!(tls.peer_certificate_der(), None);

    tls.complete_handshake().unwrap();

    assert!(!tls.is_handshaking());
    assert_eq!(tls.peer_certificate_der(), Some(leaf_der.as_ref()));

    // The connection is still perfectly usable for application data
    // afterward -- completing the handshake early doesn't consume it.
    tls.write_all(b"after handshake").unwrap();
    let mut buf = [0u8; "after handshake".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"after handshake");

    server.join().unwrap();
}

#[test]
fn pinned_anchors_with_zero_certs_is_a_hard_error() {
    let result = rusty_tls::TlsStream::new(
        std::io::Cursor::new(Vec::<u8>::new()),
        "localhost",
        &TrustPolicy::PinnedAnchors(Vec::new()),
    );
    assert!(matches!(result, Err(rusty_tls::Error::NoTrustAnchors)));
}
