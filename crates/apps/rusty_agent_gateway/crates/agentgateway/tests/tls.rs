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

use agentgateway_config::Config;
use rusty_request::TrustPolicy;
use rusty_tokio::sync::CancellationToken;
use serde_json::{Value, json};

mod common;
use common::{free_port, http, yaml_path};

/// A self-signed cert for `localhost`, written as PEM.
struct Certificate {
    dir: tempfile::TempDir,
    der: Vec<u8>,
}

fn certificate() -> Certificate {
    let issued = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("should generate a certificate");
    let cert_pem = issued.cert.pem();
    let key_pem = issued.key_pair.serialize_pem();

    let dir = tempfile::tempdir().expect("should create a temp dir");
    std::fs::write(dir.path().join("cert.pem"), &cert_pem).expect("should write the cert");
    std::fs::write(dir.path().join("key.pem"), &key_pem).expect("should write the key");

    Certificate {
        dir,
        der: issued.cert.der().to_vec(),
    }
}

/// An upstream that echoes the headers it saw.
async fn upstream() -> (u16, Arc<AtomicUsize>) {
    use rusty_axum::{Request, Router};

    let port = free_port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);

    let app = Router::new().fallback(move |request: Request| {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            let headers: serde_json::Map<String, Value> = common::header_pairs(request.headers())
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            rusty_axum::Json(json!({"path": request.path(), "headers": headers}))
        }
    });

    common::upstream(format!("127.0.0.1:{port}"), app).await;

    (port, hits)
}

/// Boot a TLS gateway in front of `upstream_port`.
async fn start(cert: &Certificate, upstream_port: u16) -> (u16, CancellationToken) {
    let port = free_port();
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
        yaml_path(cert.dir.path()),
        yaml_path(cert.dir.path()),
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = common::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = common::serve(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (port, shutdown)
}

/// Trust exactly `certs` and nothing else.
fn trusting(certs: &[&Certificate]) -> TrustPolicy {
    rusty_request::pinned_anchors(certs.iter().map(|cert| cert.der.clone()))
}

/// An HTTP/1.1 client that trusts exactly this certificate.
fn client(cert: &Certificate) -> http::Client {
    http::Client::trusting(trusting(&[cert]))
}

/// `GET path` from `https://localhost:{port}` over HTTP/2, which a TLS client
/// only gets by offering `h2` in ALPN and having the gateway pick it:
/// `rusty_hyper` refuses a connection that did not negotiate `h2`. Answers
/// the status and the body.
async fn h2_get(cert: &Certificate, port: u16, path: &str) -> Result<(u16, Vec<u8>), String> {
    let options = rusty_hyper::ConnectOptions {
        trust: trusting(&[cert]),
        ..rusty_hyper::ConnectOptions::default()
    };
    let connection = rusty_hyper::connect(&format!("https://localhost:{port}"), options)
        .await
        .map_err(|err| err.to_string())?;
    let field = rusty_hyper::HeaderField::new;
    let request = vec![
        field(":method", "GET"),
        field(":scheme", "https"),
        field(":authority", format!("localhost:{port}").as_str()),
        field(":path", path),
    ];
    let (_, response) = connection
        .client()
        .send_request(request, true)
        .await
        .map_err(|err| err.to_string())?;
    let mut response = response.await.map_err(|err| err.to_string())?;
    let status = response
        .headers
        .iter()
        .find(|h| h.name == b":status")
        .and_then(|h| std::str::from_utf8(&h.value).ok()?.parse().ok())
        .ok_or("a response without a :status")?;
    let mut body = Vec::new();
    while let Some(piece) = response.body.next().await {
        if let rusty_hyper::Body::Data(data) = piece.map_err(|err| err.to_string())? {
            body.extend(data);
        }
    }
    Ok((status, body))
}

#[rusty_tokio::test]
async fn a_request_over_tls_reaches_the_backend() {
    let cert = certificate();
    let (upstream_port, hits) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let (status, body) = h2_get(&cert, port, "/over/tls")
        .await
        .expect("the TLS handshake and request should succeed");
    assert_eq!(status, 200);
    let body: Value = serde_json::from_slice(&body).expect("upstream should answer with JSON");

    assert_eq!(body["path"], "/over/tls");
    assert_eq!(
        body["headers"]["host"].as_str().unwrap_or_default(),
        format!("127.0.0.1:{upstream_port}"),
        "an h2 client must still reach an HTTP/1.1 upstream"
    );
    assert_eq!(hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[rusty_tokio::test]
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
        .expect("should be JSON");

    assert_eq!(
        body["headers"]["x-forwarded-proto"], "https",
        "the scheme reported is the client's, not the upstream's"
    );

    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_plaintext_request_to_a_tls_port_does_not_get_through() {
    let cert = certificate();
    let (upstream_port, hits) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let result = http::Client::new()
        .get(format!("http://localhost:{port}/"))
        .send()
        .await;

    assert!(
        result.is_err(),
        "cleartext on a TLS listener must not be served, got {:?}",
        result.as_ref().map(http::Response::status)
    );
    assert_eq!(
        hits.load(Ordering::Relaxed),
        0,
        "nothing should reach the backend"
    );

    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_client_that_does_not_trust_the_certificate_is_refused() {
    // The handshake must fail on the client's own verification, which is what
    // proves the gateway is presenting a real certificate rather than
    // something the test would accept regardless.
    let cert = certificate();
    let (upstream_port, hits) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let result = http::Client::new()
        .get(format!("https://localhost:{port}/"))
        .send()
        .await;

    assert!(result.is_err(), "an untrusted certificate must be rejected");
    assert_eq!(hits.load(Ordering::Relaxed), 0);

    shutdown.cancel();
}

#[rusty_tokio::test]
async fn alpn_negotiates_http2() {
    // Over TLS, ALPN is how the HTTP version is chosen. Without advertising
    // h2 the connection silently falls back to HTTP/1.1.
    //
    // `h2_get` offers `h2` and refuses the connection unless the gateway
    // picks it, so getting an answer at all is the assertion.
    //
    // This test cannot catch a missing HTTP/2 feature in the gateway's own
    // `[dependencies]`, and it is worth knowing why: cargo unifies features
    // across a build, and a dev dependency here (`rusty_hyper`) can enable the
    // same HTTP/2 stack itself. So the *test* binary may have it while the
    // *shipped* binary only has what `[dependencies]` asks for. Removing the
    // feature once left this test green and broke the real gateway, which was
    // caught by curl against the built binary, not here.
    let cert = certificate();
    let (upstream_port, _) = upstream().await;
    let (port, shutdown) = start(&cert, upstream_port).await;

    let (status, _) = h2_get(&cert, port, "/")
        .await
        .expect("the gateway advertises h2, so a client offering only h2 should get it");
    assert_eq!(status, 200);

    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_missing_certificate_file_stops_the_gateway_booting() {
    // Rather than failing every handshake at runtime, which reads like a
    // client problem.
    let port = free_port();
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
    let err = common::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("a missing certificate must not boot");
    assert!(
        err.to_string().contains("cert.pem"),
        "the error should name the file: {err}"
    );
}

#[rusty_tokio::test]
async fn a_tls_listener_without_a_certificate_is_a_config_error() {
    let port = free_port();
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
    let err = common::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("HTTPS without a certificate cannot serve");
    assert!(
        err.to_string().contains("no `tls` certificate"),
        "got: {err}"
    );
}

#[rusty_tokio::test]
async fn two_named_certificates_on_one_port_are_served_by_name() {
    // This used to be a startup refusal: without SNI selection, serving the
    // first listener's certificate to the second listener's clients is a
    // misconfiguration nobody notices until a browser complains. Named
    // listeners are told apart now.
    let a = certificate();
    let b = certificate();
    let port = free_port();
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
        yaml_path(a.dir.path()),
        yaml_path(a.dir.path()),
        yaml_path(b.dir.path()),
        yaml_path(b.dir.path()),
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    common::build(&config, None)
        .await
        .map(|_| ())
        .expect("two named certificates on one port are told apart by SNI");
}

/// A self-signed cert for `names`, written as PEM.
fn certificate_for(names: &[&str]) -> Certificate {
    let issued = rcgen::generate_simple_self_signed(
        names.iter().map(|n| (*n).to_string()).collect::<Vec<_>>(),
    )
    .expect("should generate a certificate");
    let cert_pem = issued.cert.pem();
    let key_pem = issued.key_pair.serialize_pem();

    let dir = tempfile::tempdir().expect("should create a temp dir");
    std::fs::write(dir.path().join("cert.pem"), &cert_pem).expect("should write the cert");
    std::fs::write(dir.path().join("key.pem"), &key_pem).expect("should write the key");

    Certificate {
        dir,
        der: issued.cert.der().to_vec(),
    }
}

/// A client trusting exactly `certs` that reaches the gateway on `port`
/// under any name.
fn client_trusting(certs: &[&Certificate], port: u16) -> NamedClient {
    NamedClient {
        trust: trusting(certs),
        port,
    }
}

/// Asks for an invented hostname the way a client resolving it to the gateway
/// would: the connection goes to `127.0.0.1`, the name goes in SNI and `Host`.
/// The request is HTTP/1.1 written by hand, since what is under test is which
/// certificate (or route) the name chooses.
struct NamedClient {
    trust: TrustPolicy,
    port: u16,
}

impl NamedClient {
    /// `GET /` from `name`, or why the connection failed.
    async fn get(&self, name: &str) -> Result<Reply, String> {
        use rusty_tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let tcp = rusty_tokio::net::TcpStream::connect(("127.0.0.1", self.port))
            .await
            .map_err(|err| err.to_string())?;
        let mut tls = rusty_tls::AsyncTlsStream::new(tcp, name, &self.trust)
            .map_err(|err| err.to_string())?;
        let request = format!("GET / HTTP/1.1\r\nHost: {name}\r\nConnection: close\r\n\r\n");
        tls.write_all(request.as_bytes())
            .await
            .map_err(|err| err.to_string())?;
        let mut raw = Vec::new();
        match tls.read_to_end(&mut raw).await {
            Ok(_) => {}
            // The gateway closes the connection after the response without a
            // TLS `close_notify`. That is harmless here: the response is
            // framed, and `Reply::parse` refuses one cut short.
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => {}
            Err(err) => return Err(err.to_string()),
        }
        Reply::parse(&raw)
    }
}

/// A response read off the wire.
struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn parse(raw: &[u8]) -> Result<Reply, String> {
        let mut wire = rusty_http::sync::SyncTransport::new(raw);
        let head = wire
            .read_response_head(64 * 1024)
            .map_err(|err| err.to_string())?;
        let framing = rusty_http::body::response_framing(
            &head.headers,
            &rusty_http::Method::Get,
            head.status,
        )
        .map_err(|err| err.to_string())?;
        let body = wire.read_body(framing).map_err(|err| err.to_string())?;
        Ok(Reply {
            status: head.status.as_u16(),
            body,
        })
    }

    fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn json(&self) -> serde_json::Result<Value> {
        serde_json::from_slice(&self.body)
    }
}

/// Boot a gateway with two named HTTPS listeners on one port.
async fn start_two(
    first: (&str, &Certificate),
    second: (&str, &Certificate),
    upstream_port: u16,
) -> (u16, CancellationToken) {
    let port = free_port();
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - hostname: "{first_name}"
        protocol: HTTPS
        tls:
          cert: "{first_dir}/cert.pem"
          key: "{first_dir}/key.pem"
        routes:
          - name: first
            backends:
              - host: "127.0.0.1:{upstream_port}"
      - hostname: "{second_name}"
        protocol: HTTPS
        tls:
          cert: "{second_dir}/cert.pem"
          key: "{second_dir}/key.pem"
        routes:
          - name: second
            backends:
              - host: "127.0.0.1:{upstream_port}"
"#,
        first_name = first.0,
        first_dir = yaml_path(first.1.dir.path()),
        second_name = second.0,
        second_dir = yaml_path(second.1.dir.path()),
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = common::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = common::serve(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (port, shutdown)
}

#[rusty_tokio::test]
async fn each_hostname_is_served_its_own_certificate() {
    // The name is read off the ClientHello before the handshake starts, so
    // each client is offered a certificate it trusts and nothing else.
    let alpha = certificate_for(&["alpha.test"]);
    let beta = certificate_for(&["beta.test"]);
    let (upstream_port, hits) = upstream().await;
    let (port, shutdown) =
        start_two(("alpha.test", &alpha), ("beta.test", &beta), upstream_port).await;

    // Each client trusts only its own certificate, so a handshake that
    // succeeds against the other one's would fail here.
    let for_alpha = client_trusting(&[&alpha], port);
    let for_beta = client_trusting(&[&beta], port);

    let alpha_response = for_alpha
        .get("alpha.test")
        .await
        .expect("alpha should be served its own certificate");
    assert!(alpha_response.is_success());

    let beta_response = for_beta
        .get("beta.test")
        .await
        .expect("beta should be served its own certificate");
    assert!(beta_response.is_success());

    assert_eq!(hits.load(Ordering::Relaxed), 2);
    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_client_trusting_only_one_certificate_cannot_reach_the_other_name() {
    // The other half of the test above: proves the selection is real rather
    // than one certificate happening to satisfy both.
    let alpha = certificate_for(&["alpha.test"]);
    let beta = certificate_for(&["beta.test"]);
    let (upstream_port, _hits) = upstream().await;
    let (port, shutdown) =
        start_two(("alpha.test", &alpha), ("beta.test", &beta), upstream_port).await;

    let only_alpha = client_trusting(&[&alpha], port);
    let refused = only_alpha.get("beta.test").await;
    assert!(
        refused.is_err(),
        "beta's certificate is not one this client trusts"
    );

    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_wildcard_listener_answers_for_a_subdomain() {
    let exact = certificate_for(&["api.example.test"]);
    let wildcard = certificate_for(&["*.example.test", "other.example.test"]);
    let (upstream_port, _hits) = upstream().await;
    let (port, shutdown) = start_two(
        ("api.example.test", &exact),
        ("*.example.test", &wildcard),
        upstream_port,
    )
    .await;

    // The exact name wins over the wildcard, the same precedence route
    // hostnames follow.
    let for_exact = client_trusting(&[&exact], port);
    assert!(
        for_exact
            .get("api.example.test")
            .await
            .expect("the exact listener should answer")
            .is_success()
    );

    // A sibling name only the wildcard covers.
    let for_wildcard = client_trusting(&[&wildcard], port);
    assert!(
        for_wildcard
            .get("other.example.test")
            .await
            .expect("the wildcard listener should answer")
            .is_success()
    );

    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_name_nothing_claims_falls_back_rather_than_being_refused() {
    // Refusing would turn a working deployment into a broken one the moment a
    // second listener was added.
    let alpha = certificate_for(&["alpha.test", "stranger.test"]);
    let beta = certificate_for(&["beta.test"]);
    let (upstream_port, _hits) = upstream().await;
    let (port, shutdown) =
        start_two(("alpha.test", &alpha), ("beta.test", &beta), upstream_port).await;

    // The handshake is what this is about: no listener claims the name, so
    // routing has nothing to match and the status is beside the point.
    let client = client_trusting(&[&alpha], port);
    assert!(
        client.get("stranger.test").await.is_ok(),
        "the first certificate should have answered the handshake"
    );

    shutdown.cancel();
}

#[rusty_tokio::test]
async fn two_unnamed_certificates_on_one_port_still_fail_at_startup() {
    // A client's SNI name is what chooses between them, and neither claims
    // one: whichever sorted first would answer for both.
    let alpha = certificate_for(&["alpha.test"]);
    let beta = certificate_for(&["beta.test"]);
    let port = free_port();
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - protocol: HTTPS
        tls:
          cert: "{a}/cert.pem"
          key: "{a}/key.pem"
      - protocol: HTTPS
        tls:
          cert: "{b}/cert.pem"
          key: "{b}/key.pem"
"#,
        a = yaml_path(alpha.dir.path()),
        b = yaml_path(beta.dir.path()),
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = common::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("neither certificate can be chosen");
    assert!(err.to_string().contains("hostname"), "got: {err}");
}

#[rusty_tokio::test]
async fn two_listeners_claiming_one_name_fail_at_startup() {
    // A client asking for that name could be given either.
    let alpha = certificate_for(&["same.test"]);
    let beta = certificate_for(&["same.test"]);
    let port = free_port();
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - hostname: same.test
        protocol: HTTPS
        tls:
          cert: "{a}/cert.pem"
          key: "{a}/key.pem"
      - hostname: same.test
        protocol: HTTPS
        tls:
          cert: "{b}/cert.pem"
          key: "{b}/key.pem"
"#,
        a = yaml_path(alpha.dir.path()),
        b = yaml_path(beta.dir.path()),
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = common::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("one name cannot choose between two certificates");
    assert!(err.to_string().contains("same.test"), "got: {err}");
}

#[rusty_tokio::test]
async fn one_certificate_named_twice_is_still_one_certificate() {
    // Two listeners sharing a certificate have nothing for a name to choose
    // between, so this is not the ambiguous case.
    let shared = certificate_for(&["localhost"]);
    let (upstream_port, _hits) = upstream().await;
    let port = free_port();
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - protocol: HTTPS
        tls:
          cert: "{dir}/cert.pem"
          key: "{dir}/key.pem"
        routes:
          - backends:
              - host: "127.0.0.1:{upstream_port}"
      - protocol: HTTPS
        tls:
          cert: "{dir}/cert.pem"
          key: "{dir}/key.pem"
"#,
        dir = yaml_path(shared.dir.path()),
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let gateway = common::build(&config, None)
        .await
        .expect("one certificate twice should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = common::serve(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    assert!(
        client(&shared)
            .get(format!("https://localhost:{port}/"))
            .send()
            .await
            .is_ok(),
        "one certificate twice should still complete a handshake"
    );

    shutdown.cancel();
}

/// A TLS server that terminates with `cert` and answers every request.
///
/// The point of a passthrough test: this is the thing holding the private key,
/// not the gateway.
async fn tls_upstream(cert: &Certificate, label: &'static str) -> (u16, CancellationToken) {
    let port = free_port();
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - protocol: HTTPS
        tls:
          cert: "{dir}/cert.pem"
          key: "{dir}/key.pem"
        routes:
          - backends:
              - host: "127.0.0.1:{echo}"
"#,
        dir = yaml_path(cert.dir.path()),
        echo = upstream_for(label).await,
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    let gateway = common::build(&config, None)
        .await
        .expect("the upstream gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = common::serve(gateway, vec![addr], shutdown.clone())
        .await
        .expect("the upstream should bind");

    (port, shutdown)
}

/// A plain HTTP echo reporting `label`, behind the TLS upstream above.
async fn upstream_for(label: &'static str) -> u16 {
    use rusty_axum::Router;

    let port = free_port();
    let app =
        Router::new().fallback(move || async move { rusty_axum::Json(json!({"who": label})) });
    common::upstream(format!("127.0.0.1:{port}"), app).await;
    port
}

/// A gateway that forwards by name to the given upstreams.
async fn start_passthrough(routes: &[(&str, u16)]) -> (u16, CancellationToken) {
    let port = free_port();
    let mut yaml = format!(
        "binds:\n  - port: {port}\n    listeners:\n      - protocol: TLS\n        tcpRoutes:\n"
    );
    for (hostname, upstream_port) in routes {
        if hostname.is_empty() {
            yaml.push_str(&format!(
                "          - backends:\n              - host: \"127.0.0.1:{upstream_port}\"\n"
            ));
        } else {
            yaml.push_str(&format!(
                "          - hostnames: [\"{hostname}\"]\n            backends:\n              \
                 - host: \"127.0.0.1:{upstream_port}\"\n"
            ));
        }
    }

    let config = Config::from_yaml(&yaml).expect("config should parse");
    let gateway = common::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = common::serve(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (port, shutdown)
}

#[rusty_tokio::test]
async fn a_passthrough_listener_forwards_to_the_upstream_holding_the_key() {
    // The gateway has no certificate at all here: the one the client validates
    // is the upstream's, which is what makes this passthrough rather than
    // termination.
    let cert = certificate_for(&["alpha.test"]);
    let (upstream_port, upstream_stop) = tls_upstream(&cert, "alpha").await;
    let (port, shutdown) = start_passthrough(&[("alpha.test", upstream_port)]).await;

    let body: Value = client_trusting(&[&cert], port)
        .get("alpha.test")
        .await
        .expect("the connection should be forwarded")
        .json()
        .expect("the upstream should answer");

    assert_eq!(body["who"], "alpha");

    shutdown.cancel();
    upstream_stop.cancel();
}

#[rusty_tokio::test]
async fn each_name_is_forwarded_to_its_own_upstream() {
    let alpha_cert = certificate_for(&["alpha.test"]);
    let beta_cert = certificate_for(&["beta.test"]);
    let (alpha_port, alpha_stop) = tls_upstream(&alpha_cert, "alpha").await;
    let (beta_port, beta_stop) = tls_upstream(&beta_cert, "beta").await;

    let (port, shutdown) =
        start_passthrough(&[("alpha.test", alpha_port), ("beta.test", beta_port)]).await;

    for (name, cert, expected) in [
        ("alpha.test", &alpha_cert, "alpha"),
        ("beta.test", &beta_cert, "beta"),
    ] {
        let body: Value = client_trusting(&[cert], port)
            .get(name)
            .await
            .expect("the connection should be forwarded")
            .json()
            .expect("the upstream should answer");
        assert_eq!(body["who"], expected);
    }

    shutdown.cancel();
    alpha_stop.cancel();
    beta_stop.cancel();
}

#[rusty_tokio::test]
async fn a_name_no_route_claims_is_closed() {
    // Forwarding it to whichever route sorted first would send a connection
    // somewhere the operator never pointed that name.
    let cert = certificate_for(&["alpha.test", "stranger.test"]);
    let (upstream_port, upstream_stop) = tls_upstream(&cert, "alpha").await;
    let (port, shutdown) = start_passthrough(&[("alpha.test", upstream_port)]).await;

    let refused = client_trusting(&[&cert], port).get("stranger.test").await;
    assert!(refused.is_err(), "nothing claims that name");

    shutdown.cancel();
    upstream_stop.cancel();
}

#[rusty_tokio::test]
async fn a_catch_all_route_takes_a_name_nothing_else_claims() {
    let cert = certificate_for(&["alpha.test", "stranger.test"]);
    let (upstream_port, upstream_stop) = tls_upstream(&cert, "fallback").await;
    let (port, shutdown) = start_passthrough(&[("", upstream_port)]).await;

    let body: Value = client_trusting(&[&cert], port)
        .get("stranger.test")
        .await
        .expect("the catch-all should take it")
        .json()
        .expect("the upstream should answer");
    assert_eq!(body["who"], "fallback");

    shutdown.cancel();
    upstream_stop.cancel();
}

#[rusty_tokio::test]
async fn check_says_a_passthrough_listener_applies_no_policy() {
    // Invisible in a file that simply has no `policies:` key.
    let config = Config::from_yaml(
        r#"
binds:
  - port: 8443
    listeners:
      - protocol: TLS
        tcpRoutes:
          - hostnames: ["alpha.test"]
            backends:
              - host: "10.0.0.1:443"
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("without decrypting") && f.contains("no route policy applies")),
        "{findings:?}"
    );
}
