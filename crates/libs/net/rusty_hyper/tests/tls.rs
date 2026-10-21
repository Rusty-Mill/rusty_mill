//! TLS with ALPN `h2`, mutual TLS, and what is refused.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use rusty_hyper::{connect, ConnectOptions, Error, ServeOptions};
use rusty_tls::TrustPolicy;

fn opts(pki: &Pki) -> ConnectOptions {
    ConnectOptions {
        trust: pki.trust(),
        ..ConnectOptions::default()
    }
}

#[rusty_tokio::test]
async fn a_call_is_echoed_over_tls_with_alpn_h2() {
    let pki = pki();
    let server = start(
        listener(),
        Some(pki.acceptor(&[b"h2"])),
        ServeOptions::default(),
        echo,
    );
    let conn = connect(&url(&server, "https"), opts(&pki)).await.unwrap();
    let body = pattern(300_000);
    let r = call(conn.client(), "https", &[], body.clone())
        .await
        .unwrap();
    assert_eq!(r.body, body);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn mutual_tls_admits_a_client_with_a_certificate() {
    let pki = pki();
    let server = start(
        listener(),
        Some(pki.mtls_acceptor()),
        ServeOptions::default(),
        echo,
    );
    let o = ConnectOptions {
        identity: Some(pki.identity()),
        ..opts(&pki)
    };
    let conn = connect(&url(&server, "https"), o).await.unwrap();
    let r = call(conn.client(), "https", &[], b"hi".to_vec())
        .await
        .unwrap();
    assert_eq!(r.body, b"hi");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn mutual_tls_refuses_a_client_without_a_certificate() {
    let pki = pki();
    let server = start(
        listener(),
        Some(pki.mtls_acceptor()),
        ServeOptions::default(),
        echo,
    );
    let r = connect(&url(&server, "https"), opts(&pki)).await;
    assert!(r.is_err(), "no client certificate must not connect");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn mutual_tls_refuses_a_certificate_from_another_ca() {
    let pki = pki();
    let server = start(
        listener(),
        Some(pki.mtls_acceptor()),
        ServeOptions::default(),
        echo,
    );
    let intruder = TestCa::generate("rusty_hyper intruder CA");
    let (cert, key) = intruder.issue_leaf();
    let o = ConnectOptions {
        identity: Some(rusty_hyper::ClientIdentity {
            cert_chain_der: vec![cert],
            key_der: key,
        }),
        ..opts(&pki)
    };
    let r = connect(&url(&server, "https"), o).await;
    assert!(
        r.is_err(),
        "an untrusted client certificate must not connect"
    );
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_server_that_does_not_negotiate_h2_fails_the_client_with_alpn_not_h2() {
    let pki = pki();
    let server = start(
        listener(),
        Some(pki.acceptor(&[])),
        ServeOptions::default(),
        echo,
    );
    let r = connect(&url(&server, "https"), opts(&pki)).await;
    assert!(matches!(r, Err(Error::AlpnNotH2)), "{:?}", r.err());
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn an_untrusted_server_certificate_is_a_tls_error() {
    let pki = pki();
    let other = self::pki();
    let server = start(
        listener(),
        Some(pki.acceptor(&[b"h2"])),
        ServeOptions::default(),
        echo,
    );
    let o = ConnectOptions {
        trust: other.trust(),
        ..ConnectOptions::default()
    };
    let r = connect(&url(&server, "https"), o).await;
    assert!(
        r.is_err(),
        "a certificate from an unpinned CA must not connect"
    );
    let _ = TrustPolicy::default();
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn plain_h2c_to_a_tls_server_and_tls_to_a_plain_server_both_fail() {
    let pki = pki();
    let tls_server = start(
        listener(),
        Some(pki.acceptor(&[b"h2"])),
        ServeOptions::default(),
        echo,
    );
    let r = connect(&url(&tls_server, "http"), ConnectOptions::default()).await;
    assert!(r.is_err(), "h2c to a TLS server must not connect");
    tls_server.shutdown().await;

    let plain_server = start(listener(), None, ServeOptions::default(), echo);
    let r = connect(&url(&plain_server, "https"), opts(&pki)).await;
    assert!(r.is_err(), "TLS to a plain server must not connect");
    plain_server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_tls_client_that_offers_no_protocol_is_dropped_by_the_server() {
    use rusty_tls::AsyncTlsStream;
    use rusty_tokio::io::{AsyncWriteExt, TcpStream};

    let pki = pki();
    let server = start(
        listener(),
        Some(pki.acceptor(&[b"h2"])),
        ServeOptions::default(),
        echo,
    );
    let tcp = TcpStream::connect(server.local_addr()).await.unwrap();
    // No ALPN offer: the handshake completes, but this server speaks only h2.
    let mut tls = AsyncTlsStream::new(tcp, "127.0.0.1", &pki.trust()).unwrap();
    tls.flush().await.unwrap();
    assert_eq!(tls.negotiated_alpn_protocol(), None);
    let _ = tls.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n").await;
    let mut buf = [0u8; 64];
    let n = rusty_tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rusty_tokio::io::AsyncReadExt::read(&mut tls, &mut buf),
    )
    .await
    .expect("the server must drop it");
    // The server may send TLS alerts or close; it must never speak HTTP/2.
    assert!(matches!(n, Ok(0) | Err(_)), "got {n:?}");
    server.shutdown().await;
}
