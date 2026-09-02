//! Smoke tests for the `tokio` feature (`cargo test --features tokio`):
//! proves a request actually completes -- both `http://` and `https://`
//! -- while the whole test itself runs on a real tokio runtime
//! (`#[tokio::test]`), with no `rusty_tokio` runtime anywhere in the
//! process. `tests/client.rs`/`tests/https.rs` cover this crate's much
//! larger `rusty_tokio`-backend behavior in depth (and, with this feature
//! on, prove it's additive); this file only has to show the seam this
//! feature adds actually carries traffic, for both the plain-TCP and
//! TLS-via-compat-shim paths described in `src/tokio_compat.rs` -- plus
//! the one regression test at the bottom pinning down the run-time
//! backend choice `src/rt.rs` makes.
//!
//! Each server here is a blocking `std::net::TcpListener` on its own OS
//! thread, deliberately not tokio-based -- the point of these tests is
//! that *the client* runs entirely on real tokio; the server side just
//! needs to exist.
#![cfg(feature = "tokio")]

use rusty_request::{Body, Client};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;

/// Reads one full request (head, then exactly `Content-Length` body
/// bytes, if any -- so the client is never left writing into a socket
/// that's already been closed), sends the raw request bytes back through
/// the returned channel for a test that wants to inspect them, and
/// writes back a fixed 200 response with `body` -- once, then closes.
fn serve_one_http_response(listener: TcpListener, body: &'static [u8]) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept failed");
        let mut buf = [0u8; 4096];
        let mut seen = Vec::new();
        let head_end = loop {
            let n = stream.read(&mut buf).expect("read failed");
            assert_ne!(n, 0, "connection closed before the request head arrived");
            seen.extend_from_slice(&buf[..n]);
            if let Some(i) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&seen[..head_end]).to_string();
        let content_length: usize = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while seen.len() < head_end + content_length {
            let n = stream.read(&mut buf).expect("read failed");
            assert_ne!(n, 0, "connection closed before the request body arrived");
            seen.extend_from_slice(&buf[..n]);
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        let _ = tx.send(seen);
    });
    rx
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

    // Explicit provider for the same reason as `tests/common`'s TLS
    // server: a workspace-wide `--all-features` build compiles both
    // `ring` and `aws-lc-rs` into rustls, which makes the ambient
    // auto-detection in `ServerConfig::builder()` panic.
    let config = std::sync::Arc::new(
        ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring supports rustls's default protocol versions")
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

/// `Body::streaming_tokio` (only present with this feature) relays a real
/// tokio `AsyncRead` -- here the `impl AsyncRead for &[u8]` tokio ships
/// -- through the same shim the connector uses.
#[tokio::test]
async fn streaming_tokio_body_reaches_the_server() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test server");
    let addr: SocketAddr = listener.local_addr().unwrap();
    let received = serve_one_http_response(listener, b"ok");

    let payload: &'static [u8] = b"streamed through tokio::io::AsyncRead";
    let body = Body::streaming_tokio(Some(payload.len() as u64), move || payload);
    let resp = Client::new()
        .post(&format!("http://{addr}/upload"))
        .unwrap()
        .body(body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let raw = received.recv().expect("server never reported the request");
    assert!(
        raw.ends_with(payload),
        "request did not end with the streamed payload: {:?}",
        String::from_utf8_lossy(&raw)
    );
}

/// The regression this feature's additivity exists for: real tokio is
/// compiled in (this whole file is `cfg(feature = "tokio")`), but the
/// request is made from a task running on `rusty_tokio` -- exactly what
/// a `rusty_tokio`-based consumer sees when Cargo feature unification
/// (another crate in the build, or `--all-features` on the workspace)
/// switches this feature on for it. It must run on `rusty_tokio`'s own
/// reactor and timers; before `src/rt.rs`, it panicked with real tokio's
/// "there is no reactor running" instead. `tests/client.rs`/`https.rs`
/// cover this far more broadly; this one names the scenario explicitly.
#[test]
fn request_from_inside_rusty_tokio_still_runs_on_rusty_tokio() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test server");
    let addr: SocketAddr = listener.local_addr().unwrap();
    serve_one_http_response(listener, b"hello from rusty_tokio");

    let rt = rusty_tokio::Runtime::new().expect("rusty_tokio runtime");
    let text = rt.block_on(async {
        // The default 30s request timeout is armed on every send, so this
        // exercises the timer as well as the socket path.
        let resp = rusty_request::get(&format!("http://{addr}/"))
            .await
            .expect("request failed");
        assert_eq!(resp.status().as_u16(), 200);
        resp.text().unwrap()
    });
    assert_eq!(text, "hello from rusty_tokio");
}
