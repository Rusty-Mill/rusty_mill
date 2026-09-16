//! Async counterpart to `session_resumption.rs`: `TlsConnector::connect_async`
//! against `TlsAcceptor::accept_async`, over `rusty_tokio`.
#![cfg(feature = "rusty-tokio")]

use rusty_tls::{AsyncTlsServerStream, TlsAcceptor, TlsConnector, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt, TcpListener, TcpStream};

fn self_signed_leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

async fn connect_once(
    listener: &TcpListener,
    addr: std::net::SocketAddr,
    acceptor: &TlsAcceptor,
    connector: &TlsConnector,
) -> bool {
    let accept_fut = async {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls: AsyncTlsServerStream<TcpStream> = acceptor.accept_async(tcp).unwrap();
        let mut buf = [0u8; "ping".len()];
        tls.read_exact(&mut buf).await.unwrap();
        tls.write_all(b"pong").await.unwrap();
    };

    let connect_fut = async {
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut tls = connector.connect_async(tcp, "localhost").unwrap();
        tls.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; "pong".len()];
        tls.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"pong");
        tls.resumed_session()
    };

    let (_, resumed) = rusty_tokio::join!(accept_fut, connect_fut);
    resumed
}

#[rusty_tokio::test]
async fn async_second_connection_through_a_shared_connector_resumes() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();
    let connector = TlsConnector::new(&TrustPolicy::DangerNoVerification).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = listener.local_addr().unwrap();

    let first_resumed = connect_once(&listener, addr, &acceptor, &connector).await;
    assert!(
        !first_resumed,
        "the first connection to a server has no prior session to resume"
    );

    let second_resumed = connect_once(&listener, addr, &acceptor, &connector).await;
    assert!(
        second_resumed,
        "the second connection through the same TlsConnector should resume the first's session"
    );
}
