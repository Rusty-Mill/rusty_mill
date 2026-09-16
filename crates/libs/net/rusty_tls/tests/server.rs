//! Hermetic server-side tests — no network access, no real CA.
//!
//! Unlike `handshake.rs` (which pins a raw rustls server as the peer to
//! keep the client tests independent of this crate's own server-side
//! code), these tests exercise `TlsAcceptor`/`TlsServerStream` end to end
//! against `TlsStream`, this crate's own client — proving the two halves
//! actually interoperate, not just that each compiles in isolation.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use rcgen::KeyPair;

use rusty_tls::{TlsAcceptor, TlsServerStream, TlsStream, TrustPolicy};

fn self_signed_leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

#[test]
fn client_and_server_interoperate_end_to_end() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        let mut buf = [0u8; "hello, server".len()];
        tls.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello, server");
        tls.write_all(b"hello, client").unwrap();
    });

    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();
    tls.write_all(b"hello, server").unwrap();
    let mut buf = [0u8; "hello, client".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"hello, client");

    server.join().unwrap();
}

#[test]
fn complete_handshake_works_on_the_server_side_too() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        assert!(tls.is_handshaking());
        tls.complete_handshake().unwrap();
        assert!(!tls.is_handshaking());
        tls.write_all(b"post-handshake").unwrap();
    });

    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();
    let mut buf = [0u8; "post-handshake".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"post-handshake");

    server.join().unwrap();
}

#[test]
fn new_rejects_an_invalid_private_key() {
    let (cert_der, _) = self_signed_leaf("localhost");
    let garbage_key = vec![0u8; 16];
    let result = TlsAcceptor::new(vec![cert_der], garbage_key);
    assert!(matches!(
        result,
        Err(rusty_tls::Error::InvalidPrivateKey(_))
    ));
}

#[test]
fn new_rejects_a_key_that_does_not_match_the_certificate() {
    let (cert_der, _) = self_signed_leaf("localhost");
    // A validly-DER-encoded PKCS#8 key, but for a *different* keypair than
    // the one that signed `cert_der` -- the mismatch should surface as an
    // error building the config, not silently produce a broken acceptor.
    let unrelated_key = KeyPair::generate().unwrap().serialize_der();
    let result = TlsAcceptor::new(vec![cert_der], unrelated_key);
    assert!(result.is_err());
}
