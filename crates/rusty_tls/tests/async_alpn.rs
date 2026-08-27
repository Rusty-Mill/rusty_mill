//! Async counterpart to `alpn.rs`: `AsyncTlsStream`/`TlsAcceptor` (via
//! `accept_async`) negotiating ALPN over `rusty_tokio`.
#![cfg(feature = "rusty-tokio")]

use rusty_tls::{AsyncTlsServerStream, AsyncTlsStream, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt, TcpListener, TcpStream};

fn self_signed_leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

#[rusty_tokio::test]
async fn async_client_and_server_negotiate_a_shared_alpn_protocol() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new_with_alpn(
        vec![cert_der],
        key_der,
        vec![b"h2".to_vec(), b"http/1.1".to_vec()],
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = listener.local_addr().unwrap();

    let server = rusty_tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls: AsyncTlsServerStream<TcpStream> = acceptor.accept_async(tcp).unwrap();
        tls.complete_handshake().await.unwrap();
        let negotiated = tls.negotiated_alpn_protocol().map(|p| p.to_vec());
        tls.write_all(b"ok").await.unwrap();
        negotiated
    });

    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut tls = AsyncTlsStream::new_with_alpn(
        tcp,
        "localhost",
        &TrustPolicy::DangerNoVerification,
        vec![b"h2".to_vec()],
    )
    .unwrap();

    let mut buf = [0u8; 2];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ok");
    assert_eq!(tls.negotiated_alpn_protocol(), Some(&b"h2"[..]));

    let server_negotiated = server.await.unwrap();
    assert_eq!(server_negotiated, Some(b"h2".to_vec()));
}
