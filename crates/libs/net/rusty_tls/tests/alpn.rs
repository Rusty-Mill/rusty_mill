//! Hermetic tests for ALPN protocol negotiation
//! (`TlsStream`/`TlsAcceptor`'s `new_with_alpn` constructors and
//! `negotiated_alpn_protocol` accessors). Uses this crate's own client and
//! server types together, proving they actually interoperate.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use rusty_tls::{TlsAcceptor, TlsServerStream, TlsStream, TrustPolicy};

fn self_signed_leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

#[test]
fn client_and_server_negotiate_a_shared_alpn_protocol() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new_with_alpn(
        vec![cert_der],
        key_der,
        vec![b"h2".to_vec(), b"http/1.1".to_vec()],
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        tls.complete_handshake().unwrap();
        let negotiated = tls.negotiated_alpn_protocol().map(|p| p.to_vec());
        tls.write_all(b"ok").unwrap();
        negotiated
    });

    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new_with_alpn(
        tcp,
        "localhost",
        &TrustPolicy::DangerNoVerification,
        vec![b"h2".to_vec()],
    )
    .unwrap();
    tls.complete_handshake().unwrap();
    assert_eq!(tls.negotiated_alpn_protocol(), Some(&b"h2"[..]));

    let mut buf = [0u8; 2];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ok");

    let server_negotiated = server.join().unwrap();
    assert_eq!(server_negotiated, Some(b"h2".to_vec()));
}

#[test]
fn no_protocol_offered_means_no_protocol_negotiated() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    // Server offers no ALPN protocols at all (plain `new`).
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        tls.complete_handshake().unwrap();
        tls.negotiated_alpn_protocol().map(|p| p.to_vec())
    });

    let tcp = TcpStream::connect(addr).unwrap();
    // Client, too, offers none (plain `new`).
    let mut tls = TlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();
    tls.complete_handshake().unwrap();
    assert_eq!(tls.negotiated_alpn_protocol(), None);

    assert_eq!(server.join().unwrap(), None);
}

#[test]
fn handshake_fails_when_no_shared_alpn_protocol_exists() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new_with_alpn(vec![cert_der], key_der, vec![b"h2".to_vec()])
        .expect("valid acceptor config");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        let mut buf = [0u8; 1];
        // Expect the handshake to fail server-side too, per RFC 7301 —
        // a server with configured protocols and no overlap must send a
        // fatal alert, not silently proceed with none negotiated.
        tls.read(&mut buf)
    });

    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new_with_alpn(
        tcp,
        "localhost",
        &TrustPolicy::DangerNoVerification,
        vec![b"spdy/1".to_vec()],
    )
    .unwrap();

    let result = tls.complete_handshake();
    assert!(
        result.is_err(),
        "handshake should fail when client and server share no ALPN protocol"
    );

    assert!(server.join().unwrap().is_err());
}
