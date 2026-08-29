//! Smoke tests for the `tokio` feature (`cargo test --features tokio`):
//! proves a request actually completes -- both `http://` and `https://`
//! -- while the whole test itself runs on a real tokio runtime
//! (`#[tokio::test]`), with no `rusty_tokio` runtime anywhere in the
//! process. `tests/client.rs`/`tests/https.rs` cover this crate's much
//! larger default-feature behavior in depth; this file only has to show
//! the seam this feature adds actually carries traffic, for both the
//! plain-TCP and TLS-via-compat-shim paths described in
//! `src/tokio_compat.rs`.
//!
//! Each server here is a blocking `std::net::TcpListener` on its own OS
//! thread, deliberately not tokio-based -- the point of these tests is
//! that *the client* runs entirely on real tokio; the server side just
//! needs to exist.
#![cfg(feature = "tokio")]

use rusty_request::Client;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};

/// Reads until the request head ends, ignores its contents (every test
/// here only needs to prove a round trip happened, not exercise parsing),
/// and writes back a fixed 200 response with `body` -- once, then closes.
fn serve_one_http_response(listener: TcpListener, body: &'static [u8]) {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept failed");
        let mut buf = [0u8; 4096];
        let mut seen = Vec::new();
        loop {
            let n = stream.read(&mut buf).expect("read failed");
            assert_ne!(n, 0, "connection closed before the request head arrived");
            seen.extend_from_slice(&buf[..n]);
            if seen.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    });
}

#[tokio::test]
async fn http_request_completes_on_a_real_tokio_runtime() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test server");
    let addr: SocketAddr = listener.local_addr().unwrap();
    serve_one_http_response(listener, b"hello from real tokio");

    let resp = rusty_request::get(&format!("http://{addr}/"))
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().unwrap(), "hello from real tokio");
}

/// A self-signed CA + leaf cert for `127.0.0.1`, and a server that
/// terminates TLS for exactly one connection -- same shape as
/// `tests/common::start_tls_test_server`, just inlined so this file has
/// no dependency on the `rusty_tokio`-flavored `tests/common` module.
fn serve_one_https_response(listener: TcpListener, body: &'static [u8]) -> Vec<u8> {
    use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair};
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection, StreamOwned};

    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "rusty_request tokio-feature test CA");
    ca_params.distinguished_name = dn;
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    let leaf_params = CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap();
    let leaf_key = KeyPair::generate().unwrap();
    let leaf_cert = leaf_params.signed_by(&leaf_key, &ca_cert, &ca_key).unwrap();
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));

    let config = std::sync::Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![leaf_cert.der().clone()], key_der)
            .expect("valid test cert/key"),
    );

    std::thread::spawn(move || {
        let (tcp, _) = listener.accept().expect("accept failed");
        let conn = ServerConnection::new(config).expect("valid server config");
        let mut tls = StreamOwned::new(conn, tcp);
        let mut buf = [0u8; 4096];
        let mut seen = Vec::new();
        loop {
            let n = tls.read(&mut buf).expect("tls read failed");
            assert_ne!(n, 0, "connection closed before the request head arrived");
            seen.extend_from_slice(&buf[..n]);
            if seen.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        tls.write_all(response.as_bytes()).unwrap();
        tls.write_all(body).unwrap();
    });

    ca_cert.der().to_vec()
}

#[tokio::test]
async fn https_request_completes_on_a_real_tokio_runtime() {
    // In this workspace, `cargo test --workspace` unifies rustls's crypto-provider
    // features across every crate that depends on it -- this crate's own `ring`
    // (matching `rusty_tls`) alongside `aws-lc-rs` (pulled in by `reqwest` elsewhere
    // in the workspace, via its `rustls` feature). With both compiled in, rustls's
    // implicit single-provider auto-detection is ambiguous and `ServerConfig::builder()`
    // below panics unless a provider is installed explicitly first.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test server");
    let addr: SocketAddr = listener.local_addr().unwrap();
    let ca_der = serve_one_https_response(listener, b"hello over tls, over real tokio");

    let client = Client::builder()
        .trust_policy(rusty_request::pinned_anchors([ca_der]))
        .build();
    let resp = client
        .get(&format!("https://{addr}/"))
        .expect("failed to build request")
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().unwrap(), "hello over tls, over real tokio");
}
