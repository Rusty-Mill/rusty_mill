//! End-to-end tests for TLS termination.
//!
//! A self-signed certificate is generated per run and written to a temp
//! directory, so the gateway loads PEM off disk exactly the way it would in
//! production rather than through a test-only path. The client trusts that
//! certificate specifically — not "any certificate" — so a handshake that
//! succeeds for the wrong reason still fails the test.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind");
    listener.local_addr().expect("should have an addr").port()
}

/// A self-signed cert for `localhost`, written as PEM.
struct Certificate {
    dir: tempfile::TempDir,
    pem: String,
}

fn certificate() -> Certificate {
    let issued = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("should generate a certificate");
    let cert_pem = issued.cert.pem();
    let key_pem = issued.key_pair.serialize_pem();

    let dir = tempfile::tempdir().expect("should create a temp dir");
    std::fs::write(dir.path().join("cert.pem"), &cert_pem).expect("should write the cert");
    std::fs::write(dir.path().join("key.pem"), &key_pem).expect("should write the key");

    Certificate { dir, pem: cert_pem }
}

/// An upstream that echoes the headers it saw.
async fn upstream() -> (u16, Arc<AtomicUsize>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);

    let app = Router::new().fallback(any(move |request: Request| {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            let headers: serde_json::Map<String, Value> = request
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_string(),
                        Value::String(v.to_str().unwrap_or_default().to_string()),
                    )
                })
                .collect();
            axum::Json(json!({"path": request.uri().path(), "headers": headers}))
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("upstream should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (port, hits)
}

/// Boot a TLS gateway in front of `upstream_port`.
async fn start(cert: &Certificate, upstream_port: u16) -> (u16, CancellationToken) {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - protocol: HTTPS
        tls:
          cert: "{}/cert.pem"
          key: "{}/key.pem"
        routes:
          - name: proxy
            backends:
              - host: "127.0.0.1:{upstream_port}"
"#,
        cert.dir.path().display(),
        cert.dir.path().display(),
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (port, shutdown)
}

/// A client that trusts exactly this certificate and nothing else.
fn client(cert: &Certificate) -> reqwest::Client {
    let anchor = reqwest::Certificate::from_pem(cert.pem.as_bytes())
        .expect("the generated PEM should parse");
    reqwest::Client::builder()
        .add_root_certificate(anchor)
        .build()
        .expect("client should build")
}

#[tokio::test]
async fn a_request_over_tls_reaches_the_backend() {
    let cert = certificate();
    let (upstream_port, hits) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let body: Value = client(&cert)
        .get(format!("https://localhost:{port}/over/tls"))
        .send()
        .await
        .expect("the TLS handshake and request should succeed")
        .json()
        .await
        .expect("upstream should answer with JSON");

    assert_eq!(body["path"], "/over/tls");
    assert_eq!(
        body["headers"]["host"].as_str().unwrap_or_default(),
        format!("127.0.0.1:{upstream_port}"),
        "an h2 client must still reach an HTTP/1.1 upstream"
    );
    assert_eq!(hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn the_upstream_is_told_the_client_used_https() {
    // An upstream generating absolute URLs from this header would emit
    // http:// links into an https:// page otherwise, and browsers block that
    // as mixed content.
    let cert = certificate();
    let (upstream_port, _) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let body: Value = client(&cert)
        .get(format!("https://localhost:{port}/"))
        .send()
        .await
        .expect("request should succeed")
        .json()
        .await
        .expect("should be JSON");

    assert_eq!(
        body["headers"]["x-forwarded-proto"], "https",
        "the scheme reported is the client's, not the upstream's"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_plaintext_request_to_a_tls_port_does_not_get_through() {
    let cert = certificate();
    let (upstream_port, hits) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let result = reqwest::Client::new()
        .get(format!("http://localhost:{port}/"))
        .send()
        .await;

    assert!(
        result.is_err(),
        "cleartext on a TLS listener must not be served, got {result:?}"
    );
    assert_eq!(
        hits.load(Ordering::Relaxed),
        0,
        "nothing should reach the backend"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_client_that_does_not_trust_the_certificate_is_refused() {
    // The handshake must fail on the client's own verification, which is what
    // proves the gateway is presenting a real certificate rather than
    // something the test would accept regardless.
    let cert = certificate();
    let (upstream_port, hits) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let result = reqwest::Client::new()
        .get(format!("https://localhost:{port}/"))
        .send()
        .await;

    assert!(result.is_err(), "an untrusted certificate must be rejected");
    assert_eq!(hits.load(Ordering::Relaxed), 0);

    shutdown.cancel();
}

#[tokio::test]
async fn alpn_negotiates_http2() {
    // Over TLS, ALPN is how the HTTP version is chosen. Without advertising
    // h2 the connection silently falls back to HTTP/1.1.
    //
    // This test cannot catch a missing `hyper/http2` feature, and it is worth
    // knowing why: cargo unifies features across a build, and the dev
    // dependencies here (reqwest, rmcp's client) enable `hyper/http2`
    // themselves. So the *test* binary always has it while the *shipped*
    // binary only has what `[dependencies]` asks for. Removing the feature
    // from Cargo.toml leaves this test green and breaks the real gateway --
    // which is exactly what happened, and was caught by curl against the
    // built binary, not here.
    let cert = certificate();
    let (upstream_port, _) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let response = client(&cert)
        .get(format!("https://localhost:{port}/"))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(
        response.version(),
        reqwest::Version::HTTP_2,
        "the gateway advertises h2, so a capable client should get it"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_missing_certificate_file_stops_the_gateway_booting() {
    // Rather than failing every handshake at runtime, which reads like a
    // client problem.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - protocol: HTTPS
        tls:
          cert: /nonexistent/cert.pem
          key: /nonexistent/key.pem
        routes:
          - backends:
              - host: "127.0.0.1:1"
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("a missing certificate must not boot");
    assert!(
        err.to_string().contains("cert.pem"),
        "the error should name the file: {err}"
    );
}

#[tokio::test]
async fn a_tls_listener_without_a_certificate_is_a_config_error() {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - protocol: HTTPS
        routes:
          - backends:
              - host: "127.0.0.1:1"
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("HTTPS without a certificate cannot serve");
    assert!(
        err.to_string().contains("no `tls` certificate"),
        "got: {err}"
    );
}

#[tokio::test]
async fn two_certificates_on_one_port_are_refused_rather_than_guessed() {
    // Serving the first listener's certificate to the second listener's
    // clients is a misconfiguration nobody notices until a browser complains.
    // SNI-based selection would be the fix; refusing is the honest interim.
    let a = certificate();
    let b = certificate();
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - protocol: HTTPS
        hostname: a.example.com
        tls:
          cert: "{}/cert.pem"
          key: "{}/key.pem"
        routes:
          - backends: [{{host: "127.0.0.1:1"}}]
      - protocol: HTTPS
        hostname: b.example.com
        tls:
          cert: "{}/cert.pem"
          key: "{}/key.pem"
        routes:
          - backends: [{{host: "127.0.0.1:1"}}]
"#,
        a.dir.path().display(),
        a.dir.path().display(),
        b.dir.path().display(),
        b.dir.path().display(),
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("two certificates on one port needs SNI");
    assert!(err.to_string().contains("SNI"), "got: {err}");
}
