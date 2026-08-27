//! Hermetic handshake tests for the async adapter (feature `rusty-tokio`).
//! Mirrors `handshake.rs`'s success + rejection cases, but drives the
//! client over `rusty_tokio`'s runtime and reactor instead of blocking I/O
//! — the actual thing this adapter exists to prove works. The server side
//! stays a plain sync rustls server on a background OS thread; only the
//! client's side of the handshake is what this crate's async adapter is
//! responsible for.
#![cfg(feature = "rusty-tokio")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use rusty_tls::{AsyncTlsStream, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};

struct TestCa {
    cert: Certificate,
    key_pair: KeyPair,
}

impl TestCa {
    fn generate() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "rusty_tls async test CA");
        params.distinguished_name = dn;
        let key_pair = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        TestCa { cert, key_pair }
    }

    fn root_der(&self) -> CertificateDer<'static> {
        self.cert.der().clone()
    }

    fn issue_leaf(&self, hostname: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let params = CertificateParams::new(vec![hostname.to_string()]).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let leaf_cert = params
            .signed_by(&leaf_key, &self.cert, &self.key_pair)
            .unwrap();
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        (leaf_cert.der().clone(), key_der)
    }
}

/// Same one-shot sync echo server `handshake.rs` uses — a real TLS peer,
/// not a mock, on a background thread. The client under test is the only
/// async participant.
fn spawn_echo_server(
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
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

#[rusty_tokio::test]
async fn async_handshake_succeeds_and_round_trips_with_pinned_anchor() {
    let ca = TestCa::generate();
    let (leaf_der, leaf_key) = ca.issue_leaf("localhost");
    let (addr, server) = spawn_echo_server(leaf_der, leaf_key);

    let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![ca.root_der()]);
    let mut tls = AsyncTlsStream::new(tcp, "localhost", &policy).unwrap();

    tls.write_all(b"hello from the async adapter")
        .await
        .unwrap();
    let mut buf = [0u8; "hello from the async adapter".len()];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello from the async adapter");

    server.join().unwrap();
}

#[rusty_tokio::test]
async fn async_danger_no_verification_accepts_a_self_signed_certificate() {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
    let (addr, server) = spawn_echo_server(cert.der().clone(), key_der);

    let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
    let mut tls =
        AsyncTlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();

    tls.write_all(b"trust me, async edition").await.unwrap();
    let mut buf = [0u8; "trust me, async edition".len()];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"trust me, async edition");

    server.join().unwrap();
}

#[rusty_tokio::test]
async fn async_rejects_hostname_mismatch() {
    let ca = TestCa::generate();
    let (leaf_der, leaf_key) = ca.issue_leaf("correct-host.test");
    let (addr, _server) = spawn_echo_server(leaf_der, leaf_key);

    let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![ca.root_der()]);
    let mut tls = AsyncTlsStream::new(tcp, "wrong-host.test", &policy).unwrap();

    let result = tls.write_all(b"should not be sent").await;
    assert!(
        result.is_err(),
        "handshake should fail on hostname mismatch"
    );
}

#[rusty_tokio::test]
async fn async_rejects_untrusted_root() {
    let issuing_ca = TestCa::generate();
    let (leaf_der, leaf_key) = issuing_ca.issue_leaf("localhost");
    let (addr, _server) = spawn_echo_server(leaf_der, leaf_key);

    let unrelated_ca = TestCa::generate();
    let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![unrelated_ca.root_der()]);
    let mut tls = AsyncTlsStream::new(tcp, "localhost", &policy).unwrap();

    let result = tls.write_all(b"should not be sent").await;
    assert!(
        result.is_err(),
        "handshake should fail when the presented chain doesn't lead to a pinned anchor"
    );
}
