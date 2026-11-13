//! `TrustPolicy::PinnedCertSha256`: a server is trusted by its pinned
//! certificate fingerprint, end to end against this crate's own server.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use rusty_tls::{certificate_sha256, Error, TlsAcceptor, TlsServerStream, TlsStream, TrustPolicy};

fn self_signed_leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

/// Serves one connection with `cert_der`/`key_der`: echoes the first five
/// bytes back. Returns the address and the server thread, whose result is
/// whether the handshake completed.
fn serve_once(
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
) -> (std::net::SocketAddr, thread::JoinHandle<bool>) {
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let Ok(mut tls): Result<TlsServerStream<TcpStream>, _> = acceptor.accept(tcp) else {
            return false;
        };
        let mut buf = [0u8; 5];
        if tls.read_exact(&mut buf).is_err() {
            return false;
        }
        tls.write_all(&buf).is_ok()
    });
    (addr, server)
}

fn echo(addr: std::net::SocketAddr, name: &str, policy: &TrustPolicy) -> Result<[u8; 5], Error> {
    let tcp = TcpStream::connect(addr)?;
    let mut tls = TlsStream::new(tcp, name, policy)?;
    tls.write_all(b"hello")?;
    let mut buf = [0u8; 5];
    tls.read_exact(&mut buf)?;
    Ok(buf)
}

#[test]
fn a_pinned_certificate_is_trusted() {
    let (cert, key) = self_signed_leaf("localhost");
    let pin = certificate_sha256(&cert);
    let (addr, server) = serve_once(cert, key);

    let reply = echo(addr, "localhost", &TrustPolicy::PinnedCertSha256(vec![pin])).unwrap();
    assert_eq!(&reply, b"hello");
    assert!(server.join().unwrap());
}

#[test]
fn any_of_several_pins_is_trusted() {
    let (cert, key) = self_signed_leaf("localhost");
    let (other, _) = self_signed_leaf("localhost");
    let pins = vec![certificate_sha256(&other), certificate_sha256(&cert)];
    let (addr, server) = serve_once(cert, key);

    assert!(echo(addr, "localhost", &TrustPolicy::PinnedCertSha256(pins)).is_ok());
    assert!(server.join().unwrap());
}

#[test]
fn a_certificate_that_is_not_pinned_is_refused() {
    let (cert, key) = self_signed_leaf("localhost");
    let (other, _) = self_signed_leaf("localhost");
    let (addr, server) = serve_once(cert, key);

    let result = echo(
        addr,
        "localhost",
        &TrustPolicy::PinnedCertSha256(vec![certificate_sha256(&other)]),
    );
    assert!(result.is_err(), "an unpinned certificate was trusted");
    assert!(!server.join().unwrap());
}

#[test]
fn the_pin_stands_in_for_the_server_name() {
    // The caller keeps pins per host, so the certificate's own names are
    // not checked: one made for "localhost" is trusted under another name.
    let (cert, key) = self_signed_leaf("localhost");
    let pin = certificate_sha256(&cert);
    let (addr, server) = serve_once(cert, key);

    assert!(echo(
        addr,
        "mux.example",
        &TrustPolicy::PinnedCertSha256(vec![pin])
    )
    .is_ok());
    assert!(server.join().unwrap());
}

#[test]
fn no_pins_trust_nothing() {
    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp = TcpStream::connect(tcp_listener.local_addr().unwrap()).unwrap();
    let result = TlsStream::new(tcp, "localhost", &TrustPolicy::PinnedCertSha256(Vec::new()));
    assert!(matches!(result, Err(Error::NoTrustAnchors)));
}

#[test]
fn pin_on_first_use_then_trust_the_pin() {
    // First use: connect without verification, read the peer certificate
    // and pin its fingerprint, as a client storing it per host would.
    let (cert, key) = self_signed_leaf("localhost");
    let (addr, first) = serve_once(cert.clone(), key.clone());
    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();
    tls.complete_handshake().unwrap();
    let pin = certificate_sha256(tls.peer_certificate_der().expect("a peer certificate"));
    tls.write_all(b"hello").unwrap();
    let mut buf = [0u8; 5];
    tls.read_exact(&mut buf).unwrap();
    assert!(first.join().unwrap());
    assert_eq!(pin, certificate_sha256(&cert));

    // Later uses trust the pin.
    let (addr, second) = serve_once(cert, key);
    assert!(echo(addr, "localhost", &TrustPolicy::PinnedCertSha256(vec![pin])).is_ok());
    assert!(second.join().unwrap());
}

#[test]
fn the_fingerprint_is_sha256_of_the_der() {
    let (cert, _) = self_signed_leaf("localhost");
    let expected = ring::digest::digest(&ring::digest::SHA256, &cert);
    assert_eq!(certificate_sha256(&cert).as_slice(), expected.as_ref());
}

/// Presents `cert` but signs with an unrelated key, as someone who copied
/// a server's public certificate would.
#[derive(Debug)]
struct Impostor(std::sync::Arc<rustls::sign::CertifiedKey>);

impl rustls::server::ResolvesServerCert for Impostor {
    fn resolve(
        &self,
        _: rustls::server::ClientHello<'_>,
    ) -> Option<std::sync::Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }
}

#[test]
fn the_pinned_certificate_without_its_key_is_refused() {
    use std::sync::Arc;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let (cert, _) = self_signed_leaf("localhost");
    let (_, other_key) = self_signed_leaf("localhost");
    let pin = certificate_sha256(&cert);

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let signer = provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(other_key)))
        .unwrap();
    let certified = rustls::sign::CertifiedKey::new(vec![CertificateDer::from(cert)], signer);
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(Impostor(Arc::new(certified))));
    let config = Arc::new(config);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    // Echoes like `serve_once`, so the only thing that can fail the
    // client's echo is the client refusing the handshake.
    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let conn = rustls::ServerConnection::new(config).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        let mut buf = [0u8; 5];
        if tls.read_exact(&mut buf).is_err() {
            return false;
        }
        tls.write_all(&buf).is_ok()
    });

    let result = echo(addr, "localhost", &TrustPolicy::PinnedCertSha256(vec![pin]));
    assert!(
        result.is_err(),
        "a server without the pinned key was trusted"
    );
    assert!(!server.join().unwrap());
}
