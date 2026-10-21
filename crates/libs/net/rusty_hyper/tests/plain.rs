//! Our client against our server over plain h2c (prior knowledge).
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use rusty_hyper::{connect, ConnectOptions, Error, ServeOptions};

fn plain_echo() -> rusty_hyper::Server {
    start(listener(), None, ServeOptions::default(), echo)
}

#[rusty_tokio::test]
async fn a_unary_call_is_echoed_with_trailers() {
    let server = plain_echo();
    let conn = connect(&url(&server, "http"), ConnectOptions::default())
        .await
        .unwrap();
    let r = call(
        conn.client(),
        "http",
        &[("x-n", "7".into())],
        b"hello".to_vec(),
    )
    .await
    .unwrap();
    assert_eq!(header(&r.headers, ":status").as_deref(), Some("200"));
    assert_eq!(header(&r.headers, "x-echo").as_deref(), Some("7"));
    assert_eq!(r.body, b"hello");
    let trailers = r.trailers.expect("trailers");
    assert_eq!(header(&trailers, "grpc-status").as_deref(), Some("0"));
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_request_without_a_body_gets_an_empty_echo() {
    let server = plain_echo();
    let conn = connect(&url(&server, "http"), ConnectOptions::default())
        .await
        .unwrap();
    let r = call(conn.client(), "http", &[], Vec::new()).await.unwrap();
    assert_eq!(header(&r.headers, ":status").as_deref(), Some("200"));
    assert!(r.body.is_empty());
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn sixty_four_concurrent_calls_share_one_connection() {
    let server = plain_echo();
    let conn = connect(&url(&server, "http"), ConnectOptions::default())
        .await
        .unwrap();
    let tasks: Vec<_> = (0..64usize)
        .map(|i| {
            let h = conn.client().clone();
            rusty_tokio::spawn(async move {
                let body = pattern(1_000 + i * 97);
                let r = call(&h, "http", &[("x-n", i.to_string())], body.clone())
                    .await
                    .unwrap();
                assert_eq!(r.body, body);
                assert_eq!(header(&r.headers, "x-echo"), Some(i.to_string()));
            })
        })
        .collect();
    for t in tasks {
        t.await.unwrap();
    }
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_two_megabyte_body_crosses_flow_control_both_ways() {
    let server = plain_echo();
    let conn = connect(&url(&server, "http"), ConnectOptions::default())
        .await
        .unwrap();
    let body = pattern(2 * 1024 * 1024);
    let r = call(conn.client(), "http", &[], body.clone())
        .await
        .unwrap();
    assert_eq!(r.body, body);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn closing_a_connection_is_clean() {
    let server = plain_echo();
    let conn = connect(&url(&server, "http"), ConnectOptions::default())
        .await
        .unwrap();
    call(conn.client(), "http", &[], b"x".to_vec())
        .await
        .unwrap();
    conn.close().await.unwrap();
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_bad_url_or_scheme_is_an_error_before_any_connection() {
    let r = connect("not a url", ConnectOptions::default()).await;
    assert!(matches!(r, Err(Error::InvalidUrl(_))), "{:?}", r.err());
    let r = connect("ftp://127.0.0.1:1/", ConnectOptions::default()).await;
    assert!(
        matches!(r, Err(Error::UnsupportedScheme(_))),
        "{:?}",
        r.err()
    );
}

#[rusty_tokio::test]
async fn a_refused_connection_is_an_io_error() {
    let l = listener();
    let addr = l.local_addr().unwrap();
    drop(l);
    let r = connect(&format!("http://{addr}"), ConnectOptions::default()).await;
    assert!(matches!(r, Err(Error::Io(_))), "{:?}", r.err());
}
