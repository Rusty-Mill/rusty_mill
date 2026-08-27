//! Hermetic tests for client-certificate (mTLS) presentation on the async
//! client side (`AsyncTlsStream::new_with_client_identity`). Mirrors
//! `client_identity.rs`'s approach and CA/cert setup, but drives the client
//! over `rusty_tokio` instead of blocking I/O — the server stays a plain
//! sync rustls server on a background thread, same as `async_handshake.rs`.
#![cfg(feature = "rusty-tokio")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

use rusty_tls::{AsyncTlsStream, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};

struct TestCa {
    cert: Certificate,
    key_pair: KeyPair,
}

impl TestCa {
    fn generate(common_name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        params.distinguished_name = dn;
        let key_pair = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        TestCa { cert, key_pair }
    }

    fn root_der(&self) -> CertificateDer<'static> {
        self.cert.der().clone()
    }

    fn issue_leaf(&self, common_name: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let params = CertificateParams::new(vec![common_name.to_string()]).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let leaf_cert = params
            .signed_by(&leaf_key, &self.cert, &self.key_pair)
            .unwrap();
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        (leaf_cert.der().clone(), key_der)
    }
}

fn spawn_client_auth_server(
    server_cert_der: CertificateDer<'static>,
    server_key_der: PrivateKeyDer<'static>,
    client_ca_root: CertificateDer<'static>,
) -> (SocketAddr, thread::JoinHandle<Option<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let mut roots = RootCertStore::empty();
    roots.add(client_ca_root).unwrap();
    let client_verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .unwrap();
    let config = Arc::new(
        ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(vec![server_cert_der], server_key_der)
            .expect("valid test cert/key"),
    );

    let handle = thread::spawn(move || {
        let (tcp, _) = listener.accept().ok()?;
        let conn = ServerConnection::new(config).ok()?;
        let mut tls = StreamOwned::new(conn, tcp);
        let mut buf = [0u8; 1024];
        let n = tls.read(&mut buf).ok()?;
        let _ = tls.write_all(&buf[..n]);
        Some(buf[..n].to_vec())
    });

    (addr, handle)
}

#[rusty_tokio::test]
async fn async_client_presents_a_client_certificate_the_server_accepts() {
    let server_ca = TestCa::generate("rusty_tls async test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");

    let client_ca = TestCa::generate("rusty_tls async test client CA");
    let (client_leaf_der, client_leaf_key) = client_ca.issue_leaf("test-client");

    let (addr, server) =
        spawn_client_auth_server(server_leaf_der, server_leaf_key, client_ca.root_der());

    let tcp = rusty_tokio::io::TcpStream::connect(addr).await.unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![server_ca.root_der()]);
    let mut tls = AsyncTlsStream::new_with_client_identity(
        tcp,
        "localhost",
        &policy,
        vec![client_leaf_der.to_vec()],
        client_leaf_key.secret_der().to_vec(),
    )
    .unwrap();

    tls.write_all(b"hello with an async client cert")
        .await
        .unwrap();
    let mut buf = [0u8; "hello with an async client cert".len()];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello with an async client cert");

    let received = server.join().unwrap();
    assert_eq!(
        received.as_deref(),
        Some(&b"hello with an async client cert"[..])
    );
}
