//! Hermetic tests for server-side client-certificate verification
//! (`TlsAcceptor::new_with_client_auth`). Unlike `client_identity.rs`
//! (which pins a raw rustls server to test the client side in isolation),
//! these tests exercise this crate's own client *and* server mTLS support
//! together — proving #9 and #10 actually interoperate, not just that each
//! compiles in isolation.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::CertificateDer;

use rusty_tls::{TlsAcceptor, TlsServerStream, TlsStream, TrustPolicy};

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

    fn root_der(&self) -> Vec<u8> {
        self.cert.der().to_vec()
    }

    fn issue_leaf(&self, common_name: &str) -> (Vec<u8>, Vec<u8>) {
        let params = CertificateParams::new(vec![common_name.to_string()]).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let leaf_cert = params
            .signed_by(&leaf_key, &self.cert, &self.key_pair)
            .unwrap();
        (leaf_cert.der().to_vec(), leaf_key.serialize_der())
    }
}

#[test]
fn server_verifies_a_client_certificate_it_trusts() {
    let server_ca = TestCa::generate("rusty_tls mTLS test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");

    let client_ca = TestCa::generate("rusty_tls mTLS test client CA");
    let (client_leaf_der, client_leaf_key) = client_ca.issue_leaf("test-client");

    let acceptor = TlsAcceptor::new_with_client_auth(
        vec![server_leaf_der],
        server_leaf_key,
        vec![client_ca.root_der()],
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        let mut buf = [0u8; "hello with mTLS".len()];
        tls.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello with mTLS");
        tls.write_all(b"verified, welcome").unwrap();
    });

    let tcp = TcpStream::connect(addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![CertificateDer::from(server_ca.root_der())]);
    let mut tls = TlsStream::new_with_client_identity(
        tcp,
        "localhost",
        &policy,
        vec![client_leaf_der],
        client_leaf_key,
    )
    .unwrap();

    tls.write_all(b"hello with mTLS").unwrap();
    let mut buf = [0u8; "verified, welcome".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"verified, welcome");

    server.join().unwrap();
}

#[test]
fn server_rejects_a_connection_with_no_client_certificate() {
    let server_ca = TestCa::generate("rusty_tls mTLS test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");
    let client_ca = TestCa::generate("rusty_tls mTLS test client CA");

    let acceptor = TlsAcceptor::new_with_client_auth(
        vec![server_leaf_der],
        server_leaf_key,
        vec![client_ca.root_der()],
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        let mut buf = [0u8; 1];
        // Expect the handshake itself to fail (no client cert presented,
        // but the server mandates one) rather than any application data
        // successfully arriving.
        matches!(tls.read(&mut buf), Ok(0) | Err(_))
    });

    let tcp = TcpStream::connect(addr).unwrap();
    // Plain `new` — no client identity, but the acceptor requires one.
    let policy = TrustPolicy::PinnedAnchors(vec![CertificateDer::from(server_ca.root_der())]);
    let mut tls = TlsStream::new(tcp, "localhost", &policy).unwrap();

    // As in `client_identity.rs`'s equivalent rejection test: the failure
    // may only surface on a follow-up read, not synchronously from this
    // write.
    let write_result = tls.write_all(b"should not be accepted");
    let mut buf = [0u8; 1];
    let read_result = tls.read(&mut buf);
    assert!(
        write_result.is_err() || read_result.is_err(),
        "connection should fail when the acceptor requires a client certificate and none is presented"
    );

    assert!(server.join().unwrap());
}

#[test]
fn server_rejects_a_client_certificate_from_an_untrusted_ca() {
    let server_ca = TestCa::generate("rusty_tls mTLS test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");

    let trusted_client_ca = TestCa::generate("rusty_tls mTLS trusted client CA");
    let untrusted_client_ca = TestCa::generate("rusty_tls mTLS untrusted client CA");
    let (client_leaf_der, client_leaf_key) = untrusted_client_ca.issue_leaf("test-client");

    let acceptor = TlsAcceptor::new_with_client_auth(
        vec![server_leaf_der],
        server_leaf_key,
        vec![trusted_client_ca.root_der()],
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        let mut buf = [0u8; 1];
        matches!(tls.read(&mut buf), Ok(0) | Err(_))
    });

    let tcp = TcpStream::connect(addr).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![CertificateDer::from(server_ca.root_der())]);
    let mut tls = TlsStream::new_with_client_identity(
        tcp,
        "localhost",
        &policy,
        vec![client_leaf_der],
        client_leaf_key,
    )
    .unwrap();

    let write_result = tls.write_all(b"should not be accepted");
    let mut buf = [0u8; 1];
    let read_result = tls.read(&mut buf);
    assert!(
        write_result.is_err() || read_result.is_err(),
        "connection should fail when the client certificate doesn't chain to a trusted client CA"
    );

    assert!(server.join().unwrap());
}

#[test]
fn new_with_client_auth_rejects_empty_client_ca_roots() {
    let server_ca = TestCa::generate("rusty_tls mTLS test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");

    let result = TlsAcceptor::new_with_client_auth(vec![server_leaf_der], server_leaf_key, vec![]);
    assert!(matches!(
        result,
        Err(rusty_tls::Error::InvalidClientCaRoots(_))
    ));
}
