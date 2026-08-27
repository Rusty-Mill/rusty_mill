//! Hermetic tests for the async server adapter (feature `rusty-tokio`).
//! Mirrors `tests/server.rs`'s end-to-end interop shape, but with both
//! halves async — proving `AsyncTlsServerStream` actually interoperates
//! with `AsyncTlsStream`, not just that it compiles in isolation.
#![cfg(feature = "rusty-tokio")]

use rusty_tls::{AsyncTlsServerStream, AsyncTlsStream, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt, TcpListener, TcpStream};

fn self_signed_leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

#[rusty_tokio::test]
async fn async_client_and_async_server_interoperate_end_to_end() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = listener.local_addr().unwrap();

    let server = rusty_tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls: AsyncTlsServerStream<TcpStream> = acceptor.accept_async(tcp).unwrap();
        let mut buf = [0u8; "hello, async server".len()];
        tls.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello, async server");
        tls.write_all(b"hello, async client").await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut tls =
        AsyncTlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();
    tls.write_all(b"hello, async server").await.unwrap();
    let mut buf = [0u8; "hello, async client".len()];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello, async client");

    server.await.unwrap();
}

#[rusty_tokio::test]
async fn complete_handshake_works_on_the_async_server_side_too() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = listener.local_addr().unwrap();

    let server = rusty_tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls: AsyncTlsServerStream<TcpStream> = acceptor.accept_async(tcp).unwrap();
        assert!(tls.is_handshaking());
        tls.complete_handshake().await.unwrap();
        assert!(!tls.is_handshaking());
        tls.write_all(b"post-handshake").await.unwrap();
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut tls =
        AsyncTlsStream::new(tcp, "localhost", &TrustPolicy::DangerNoVerification).unwrap();
    let mut buf = [0u8; "post-handshake".len()];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"post-handshake");

    server.await.unwrap();
}
