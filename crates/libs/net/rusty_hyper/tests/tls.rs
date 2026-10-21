//! TLS with ALPN `h2`, mutual TLS, and what is refused.

mod common;

use common::*;
use rusty_hyper::{connect, serve, ConnectOptions, Error, ServeOptions};
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
    let server = serve(
        listener(),
        Some(pki.acceptor(&[b"h2"])),
        ServeOptions::default(),
        echo,
    );
    let conn = connect(&url(&server, "https"), opts(&pki)).await.unwrap();
    let body = pattern(300_000);
    let r = call(conn.client(), "https", &[], body.clone()).await.unwrap();
    assert_eq!(r.body, body);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn mutual_tls_admits_a_client_with_a_certificate() {
    let pki = pki();
    let server = serve(
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
    let r = call(conn.client(), "https", &[], b"hi".to_vec()).await.unwrap();
    assert_eq!(r.body, b"hi");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn mutual_tls_refuses_a_client_without_a_certificate() {
    let pki = pki();
    let server = serve(
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
    let server = serve(
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
    assert!(r.is_err(), "an untrusted client certificate must not connect");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_server_that_does_not_negotiate_h2_fails_the_client_with_alpn_not_h2() {
    let pki = pki();
    let server = serve(
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
    let server = serve(
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
    assert!(r.is_err(), "a certificate from an unpinned CA must not connect");
    let _ = TrustPolicy::default();
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn plain_h2c_to_a_tls_server_and_tls_to_a_plain_server_both_fail() {
    let pki = pki();
    let tls_server = serve(
        listener(),
        Some(pki.acceptor(&[b"h2"])),
        ServeOptions::default(),
        echo,
    );
    let r = connect(&url(&tls_server, "http"), ConnectOptions::default()).await;
    assert!(r.is_err(), "h2c to a TLS server must not connect");
    tls_server.shutdown().await;

    let plain_server = serve(listener(), None, ServeOptions::default(), echo);
    let r = connect(&url(&plain_server, "https"), opts(&pki)).await;
    assert!(r.is_err(), "TLS to a plain server must not connect");
    plain_server.shutdown().await;
}
