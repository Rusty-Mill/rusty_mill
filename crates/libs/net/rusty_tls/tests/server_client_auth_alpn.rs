//! A server that requires a client certificate *and* negotiates ALPN
//! (`TlsAcceptor::new_with_client_auth_and_alpn`), against this crate's own
//! clients (`new_with_client_identity_and_alpn`). A gRPC server behind mutual
//! TLS needs both on one connection; `new_with_client_auth` and
//! `new_with_alpn` each gave only one.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::CertificateDer;

use rusty_tls::{Error, TlsAcceptor, TlsServerStream, TlsStream, TrustPolicy};

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

/// A server CA with its leaf, and a client CA with a leaf, plus an acceptor
/// that requires client certificates from the client CA and offers `h2`.
struct Fixture {
    server_ca: TestCa,
    client_ca: TestCa,
    client_leaf: (Vec<u8>, Vec<u8>),
    acceptor: TlsAcceptor,
}

fn fixture(alpn: Vec<Vec<u8>>) -> Fixture {
    let server_ca = TestCa::generate("rusty_tls mTLS+ALPN test server CA");
    let (server_leaf_der, server_leaf_key) = server_ca.issue_leaf("localhost");
    let client_ca = TestCa::generate("rusty_tls mTLS+ALPN test client CA");
    let client_leaf = client_ca.issue_leaf("test-client");
    let acceptor = TlsAcceptor::new_with_client_auth_and_alpn(
        vec![server_leaf_der],
        server_leaf_key,
        vec![client_ca.root_der()],
        alpn,
    )
    .unwrap();
    Fixture {
        server_ca,
        client_ca,
        client_leaf,
        acceptor,
    }
}

fn policy(f: &Fixture) -> TrustPolicy {
    TrustPolicy::PinnedAnchors(vec![CertificateDer::from(f.server_ca.root_der())])
}

/// What the server saw on one connection.
struct Seen {
    alpn: Option<Vec<u8>>,
    client_cert: Option<Vec<u8>>,
}

/// Accepts one connection, completes the handshake and reports what it saw;
/// `None` when the handshake failed.
fn serve_once(acceptor: TlsAcceptor, listener: TcpListener) -> thread::JoinHandle<Option<Seen>> {
    thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        tls.complete_handshake().ok()?;
        let seen = Seen {
            alpn: tls.negotiated_alpn_protocol().map(<[u8]>::to_vec),
            client_cert: tls.peer_certificate_der().map(<[u8]>::to_vec),
        };
        let mut buf = [0u8; 4];
        tls.read_exact(&mut buf).ok()?;
        tls.write_all(&buf).ok()?;
        Some(seen)
    })
}

fn echo(tls: &mut TlsStream<TcpStream>) -> std::io::Result<()> {
    tls.write_all(b"ping")?;
    let mut buf = [0u8; 4];
    tls.read_exact(&mut buf)?;
    assert_eq!(&buf, b"ping");
    Ok(())
}

#[test]
fn the_server_sees_both_the_negotiated_protocol_and_the_client_certificate() {
    let f = fixture(vec![b"h2".to_vec()]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = serve_once(f.acceptor.clone(), listener);

    let (cert, key) = f.client_leaf.clone();
    let mut tls = TlsStream::new_with_client_identity_and_alpn(
        TcpStream::connect(addr).unwrap(),
        "localhost",
        &policy(&f),
        vec![cert.clone()],
        key,
        vec![b"h2".to_vec()],
    )
    .unwrap();
    echo(&mut tls).unwrap();
    assert_eq!(tls.negotiated_alpn_protocol(), Some(&b"h2"[..]));

    let seen = server.join().unwrap().expect("handshake succeeds");
    assert_eq!(seen.alpn.as_deref(), Some(&b"h2"[..]));
    assert_eq!(seen.client_cert, Some(cert));
}

#[test]
fn a_client_that_offers_no_protocol_still_connects_with_none_negotiated() {
    let f = fixture(vec![b"h2".to_vec()]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = serve_once(f.acceptor.clone(), listener);

    let (cert, key) = f.client_leaf.clone();
    let mut tls = TlsStream::new_with_client_identity(
        TcpStream::connect(addr).unwrap(),
        "localhost",
        &policy(&f),
        vec![cert],
        key,
    )
    .unwrap();
    echo(&mut tls).unwrap();
    assert_eq!(tls.negotiated_alpn_protocol(), None);

    let seen = server.join().unwrap().expect("handshake succeeds");
    assert_eq!(seen.alpn, None);
    assert!(seen.client_cert.is_some());
}

#[test]
fn a_client_offering_only_other_protocols_is_refused() {
    let f = fixture(vec![b"h2".to_vec()]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = serve_once(f.acceptor.clone(), listener);

    let (cert, key) = f.client_leaf.clone();
    let mut tls = TlsStream::new_with_client_identity_and_alpn(
        TcpStream::connect(addr).unwrap(),
        "localhost",
        &policy(&f),
        vec![cert],
        key,
        vec![b"http/1.1".to_vec()],
    )
    .unwrap();
    assert!(echo(&mut tls).is_err(), "no protocol in common");
    assert!(server.join().unwrap().is_none());
}

#[test]
fn a_client_without_a_certificate_is_still_refused_when_alpn_matches() {
    let f = fixture(vec![b"h2".to_vec()]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = serve_once(f.acceptor.clone(), listener);

    let mut tls = TlsStream::new_with_alpn(
        TcpStream::connect(addr).unwrap(),
        "localhost",
        &policy(&f),
        vec![b"h2".to_vec()],
    )
    .unwrap();
    assert!(echo(&mut tls).is_err(), "the server requires a certificate");
    assert!(server.join().unwrap().is_none());
}

#[test]
fn a_certificate_from_an_untrusted_ca_is_refused_when_alpn_matches() {
    let f = fixture(vec![b"h2".to_vec()]);
    let other = TestCa::generate("rusty_tls mTLS+ALPN untrusted client CA");
    let (cert, key) = other.issue_leaf("intruder");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = serve_once(f.acceptor.clone(), listener);

    let mut tls = TlsStream::new_with_client_identity_and_alpn(
        TcpStream::connect(addr).unwrap(),
        "localhost",
        &policy(&f),
        vec![cert],
        key,
        vec![b"h2".to_vec()],
    )
    .unwrap();
    assert!(echo(&mut tls).is_err(), "the CA is not trusted");
    assert!(server.join().unwrap().is_none());
    drop(f.client_ca);
}

#[test]
fn an_empty_protocol_list_negotiates_nothing_but_still_requires_a_certificate() {
    let f = fixture(vec![]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = serve_once(f.acceptor.clone(), listener);

    let (cert, key) = f.client_leaf.clone();
    let mut tls = TlsStream::new_with_client_identity_and_alpn(
        TcpStream::connect(addr).unwrap(),
        "localhost",
        &policy(&f),
        vec![cert],
        key,
        vec![b"h2".to_vec()],
    )
    .unwrap();
    echo(&mut tls).unwrap();
    assert_eq!(tls.negotiated_alpn_protocol(), None);
    let seen = server.join().unwrap().expect("handshake succeeds");
    assert_eq!(seen.alpn, None);
    assert!(seen.client_cert.is_some());
}

#[test]
fn empty_client_ca_roots_and_a_bad_key_are_errors_as_in_new_with_client_auth() {
    let ca = TestCa::generate("rusty_tls mTLS+ALPN test server CA");
    let (cert, key) = ca.issue_leaf("localhost");
    let no_roots = TlsAcceptor::new_with_client_auth_and_alpn(
        vec![cert.clone()],
        key,
        vec![],
        vec![b"h2".to_vec()],
    );
    assert!(matches!(no_roots, Err(Error::InvalidClientCaRoots(_))));

    let bad_key = TlsAcceptor::new_with_client_auth_and_alpn(
        vec![cert],
        b"not a key".to_vec(),
        vec![ca.root_der()],
        vec![b"h2".to_vec()],
    );
    assert!(matches!(bad_key, Err(Error::InvalidPrivateKey(_))));
}

#[cfg(feature = "rusty-tokio")]
mod asynchronous {
    use super::*;
    use rusty_tls::{AsyncTlsServerStream, AsyncTlsStream};
    use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt, TcpListener, TcpStream};

    #[rusty_tokio::test]
    async fn the_async_server_negotiates_the_protocol_while_requiring_a_client_certificate() {
        let f = fixture(vec![b"h2".to_vec()]);
        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = f.acceptor.clone();
        let server = rusty_tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut tls: AsyncTlsServerStream<TcpStream> = acceptor.accept_async(tcp).unwrap();
            tls.complete_handshake().await.unwrap();
            // `AsyncTlsServerStream` has no `peer_certificate_der`; that the
            // handshake completes at all shows the certificate was accepted.
            let alpn = tls.negotiated_alpn_protocol().map(<[u8]>::to_vec);
            let mut buf = [0u8; 4];
            tls.read_exact(&mut buf).await.unwrap();
            tls.write_all(&buf).await.unwrap();
            alpn
        });

        let (cert, key) = f.client_leaf.clone();
        let mut tls = AsyncTlsStream::new_with_client_identity_and_alpn(
            TcpStream::connect(addr).await.unwrap(),
            "localhost",
            &policy(&f),
            vec![cert],
            key,
            vec![b"h2".to_vec()],
        )
        .unwrap();
        tls.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        tls.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        assert_eq!(tls.negotiated_alpn_protocol(), Some(&b"h2"[..]));

        assert_eq!(server.await.unwrap().as_deref(), Some(&b"h2"[..]));
    }
}
