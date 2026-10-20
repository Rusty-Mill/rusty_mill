//! mTLS and ALPN on one client connection (`TlsStream::new_with_client_identity_and_alpn`):
//! the server requires a client certificate *and* negotiates `h2`, as a gRPC
//! server behind mutual TLS does. A plain sync rustls server plays the peer.

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

use rusty_tls::{Error, TlsStream, TrustPolicy};

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

/// What the server saw: the bytes it read, the ALPN protocol it settled on,
/// and whether the peer presented a certificate.
#[derive(Debug, PartialEq, Eq)]
struct ServerSaw {
    bytes: Vec<u8>,
    alpn: Option<Vec<u8>>,
    peer_certificates: usize,
}

/// A plain sync rustls server that *requires* a client certificate signed by
/// `client_ca_root` and offers `server_alpn`, on a background thread. `None`
/// means the handshake never completed.
fn spawn_server(
    server_cert_der: CertificateDer<'static>,
    server_key_der: PrivateKeyDer<'static>,
    client_ca_root: CertificateDer<'static>,
    server_alpn: Vec<Vec<u8>>,
) -> (SocketAddr, thread::JoinHandle<Option<ServerSaw>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let mut roots = RootCertStore::empty();
    roots.add(client_ca_root).unwrap();
    let client_verifier = WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports the safe default protocol versions")
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(vec![server_cert_der], server_key_der)
            .expect("valid test cert/key");
    config.alpn_protocols = server_alpn;
    let config = Arc::new(config);

    let handle = thread::spawn(move || {
        let (tcp, _) = listener.accept().ok()?;
        let conn = ServerConnection::new(config).ok()?;
        let mut tls = StreamOwned::new(conn, tcp);
        let mut buf = [0u8; 1024];
        let n = tls.read(&mut buf).ok()?;
        let _ = tls.write_all(&buf[..n]);
        Some(ServerSaw {
            bytes: buf[..n].to_vec(),
            alpn: tls.conn.alpn_protocol().map(<[u8]>::to_vec),
            peer_certificates: tls.conn.peer_certificates().map_or(0, <[_]>::len),
        })
    });

    (addr, handle)
}

struct Fixture {
    server_ca: TestCa,
    client_ca: TestCa,
    addr: SocketAddr,
    server: thread::JoinHandle<Option<ServerSaw>>,
    client_leaf: (CertificateDer<'static>, PrivateKeyDer<'static>),
}

fn fixture(server_alpn: Vec<Vec<u8>>) -> Fixture {
    let server_ca = TestCa::generate("rusty_tls test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");
    let client_ca = TestCa::generate("rusty_tls test client CA");
    let client_leaf = client_ca.issue_leaf("test-client");
    let (addr, server) = spawn_server(
        server_leaf_der,
        server_leaf_key,
        client_ca.root_der(),
        server_alpn,
    );
    Fixture {
        server_ca,
        client_ca,
        addr,
        server,
        client_leaf,
    }
}

#[test]
fn offers_alpn_and_presents_a_client_certificate_in_one_handshake() {
    let f = fixture(vec![b"h2".to_vec()]);
    let tcp = TcpStream::connect(f.addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![f.server_ca.root_der()]);
    let mut tls = TlsStream::new_with_client_identity_and_alpn(
        tcp,
        "localhost",
        &policy,
        vec![f.client_leaf.0.to_vec()],
        f.client_leaf.1.secret_der().to_vec(),
        vec![b"h2".to_vec(), b"http/1.1".to_vec()],
    )
    .unwrap();
    assert_eq!(
        tls.negotiated_alpn_protocol(),
        None,
        "nothing before the handshake"
    );

    tls.write_all(b"mtls + alpn").unwrap();
    let mut buf = [0u8; 11];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"mtls + alpn");
    assert_eq!(tls.negotiated_alpn_protocol(), Some(&b"h2"[..]));

    let saw = f.server.join().unwrap().expect("the handshake completed");
    assert_eq!(saw.bytes, b"mtls + alpn");
    assert_eq!(
        saw.alpn.as_deref(),
        Some(&b"h2"[..]),
        "the server saw the offer"
    );
    assert_eq!(saw.peer_certificates, 1, "and the client certificate");
}

#[test]
fn a_server_with_no_alpn_leaves_the_protocol_unnegotiated_but_the_certificate_still_works() {
    let f = fixture(Vec::new());
    let tcp = TcpStream::connect(f.addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![f.server_ca.root_der()]);
    let mut tls = TlsStream::new_with_client_identity_and_alpn(
        tcp,
        "localhost",
        &policy,
        vec![f.client_leaf.0.to_vec()],
        f.client_leaf.1.secret_der().to_vec(),
        vec![b"h2".to_vec()],
    )
    .unwrap();
    tls.write_all(b"x").unwrap();
    let mut buf = [0u8; 1];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(tls.negotiated_alpn_protocol(), None);
    let saw = f.server.join().unwrap().expect("the handshake completed");
    assert_eq!(saw.peer_certificates, 1);
}

#[test]
fn a_client_certificate_from_the_wrong_ca_is_still_refused() {
    let f = fixture(vec![b"h2".to_vec()]);
    let other_ca = TestCa::generate("an unrelated CA");
    let (bad_cert, bad_key) = other_ca.issue_leaf("intruder");
    let tcp = TcpStream::connect(f.addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![f.server_ca.root_der()]);
    let mut tls = TlsStream::new_with_client_identity_and_alpn(
        tcp,
        "localhost",
        &policy,
        vec![bad_cert.to_vec()],
        bad_key.secret_der().to_vec(),
        vec![b"h2".to_vec()],
    )
    .unwrap();
    let _ = tls.write_all(b"let me in");
    let mut buf = [0u8; 16];
    assert!(tls.read(&mut buf).is_err() || f.server.join().unwrap().is_none());
    drop(f.client_ca);
}

#[test]
fn an_invalid_private_key_is_an_error_not_a_panic() {
    let f = fixture(vec![b"h2".to_vec()]);
    let tcp = TcpStream::connect(f.addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![f.server_ca.root_der()]);
    let result = TlsStream::new_with_client_identity_and_alpn(
        tcp,
        "localhost",
        &policy,
        vec![f.client_leaf.0.to_vec()],
        vec![1, 2, 3],
        vec![b"h2".to_vec()],
    );
    assert!(matches!(result, Err(Error::InvalidPrivateKey(_))));
}

#[test]
fn an_invalid_server_name_is_an_error() {
    let f = fixture(vec![b"h2".to_vec()]);
    let tcp = TcpStream::connect(f.addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![f.server_ca.root_der()]);
    let result = TlsStream::new_with_client_identity_and_alpn(
        tcp,
        "not a host name",
        &policy,
        vec![f.client_leaf.0.to_vec()],
        f.client_leaf.1.secret_der().to_vec(),
        vec![b"h2".to_vec()],
    );
    assert!(matches!(result, Err(Error::InvalidServerName(_))));
}
