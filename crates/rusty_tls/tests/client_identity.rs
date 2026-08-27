//! Hermetic tests for client-certificate (mTLS) presentation on the client
//! side (`TlsStream::new_with_client_identity`). Mirrors `async_handshake.rs`'s
//! approach: a plain sync rustls server (not this crate's own
//! `TlsAcceptor`, which doesn't verify client certs yet) plays the peer, so
//! these tests are independent of the server-side mTLS gap tracked
//! separately in issue #10.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

use rusty_tls::{TlsStream, TrustPolicy};

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

/// A plain sync rustls server that *requires* a client certificate signed
/// by `client_ca_root`, on a background thread. Returns whatever bytes it
/// read (or `None` if the handshake never completed), so tests can
/// distinguish "rejected, as expected" from "accidentally worked."
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

#[test]
fn presents_a_client_certificate_the_server_accepts() {
    let server_ca = TestCa::generate("rusty_tls test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");

    let client_ca = TestCa::generate("rusty_tls test client CA");
    let (client_leaf_der, client_leaf_key) = client_ca.issue_leaf("test-client");

    let (addr, server) =
        spawn_client_auth_server(server_leaf_der, server_leaf_key, client_ca.root_der());

    let tcp = TcpStream::connect(addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![server_ca.root_der()]);
    let mut tls = TlsStream::new_with_client_identity(
        tcp,
        "localhost",
        &policy,
        vec![client_leaf_der.to_vec()],
        client_leaf_key.secret_der().to_vec(),
    )
    .unwrap();

    tls.write_all(b"hello with a client cert").unwrap();
    let mut buf = [0u8; "hello with a client cert".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"hello with a client cert");

    let received = server.join().unwrap();
    assert_eq!(received.as_deref(), Some(&b"hello with a client cert"[..]));
}

#[test]
fn rejects_when_no_client_certificate_is_presented() {
    let server_ca = TestCa::generate("rusty_tls test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");
    let client_ca = TestCa::generate("rusty_tls test client CA");

    let (addr, server) =
        spawn_client_auth_server(server_leaf_der, server_leaf_key, client_ca.root_der());

    let tcp = TcpStream::connect(addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![server_ca.root_der()]);
    // Plain `new` — no client identity presented, but the server mandates one.
    let mut tls = TlsStream::new(tcp, "localhost", &policy).unwrap();

    // The client's own handshake state considers itself done as soon as it
    // sends its (empty) Certificate + Finished — mandatory-client-auth
    // rejection is a server-side policy decision the client only learns
    // about via the server's fatal alert on a later read, not synchronously
    // from this write. So `write_all` alone may report success; the actual
    // rejection surfaces on the read that follows.
    let write_result = tls.write_all(b"should not be accepted");
    let mut buf = [0u8; 1];
    let read_result = tls.read(&mut buf);
    assert!(
        write_result.is_err() || read_result.is_err(),
        "connection should fail when the server requires a client certificate and none is presented"
    );

    let received = server.join().unwrap();
    assert_eq!(received, None);
}
