//! End-to-end tests for `host` backend proxying.
//!
//! A real upstream HTTP server is stood up per test; it echoes back the
//! request line and headers it saw as JSON, so the assertions are about what
//! actually arrived upstream rather than what the gateway believed it sent.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentgateway_config::Config;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

mod common;
use common::free_port;

/// An upstream that reports what it received.
struct Upstream {
    port: u16,
    hits: Arc<AtomicUsize>,
}

/// Start an echo upstream labelled `name`.
async fn upstream(name: &'static str) -> Upstream {
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
            axum::Json(json!({
                "upstream": name,
                "method": request.method().as_str(),
                "path": request.uri().path(),
                "query": request.uri().query().unwrap_or_default(),
                "headers": headers,
            }))
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("upstream should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Upstream { port, hits }
}

/// Boot a gateway from a route body, returning its base URL.
async fn start(route: &str) -> (String, CancellationToken) {
    start_with_inventory(route, "").await
}

/// The same, with a `services:`/`workloads:` inventory spliced in.
async fn start_with_inventory(route: &str, inventory: &str) -> (String, CancellationToken) {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
{route}
{inventory}
"#
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

    (format!("http://127.0.0.1:{port}"), shutdown)
}

async fn get(url: &str) -> Value {
    reqwest::get(url)
        .await
        .expect("request should reach the gateway")
        .json()
        .await
        .expect("upstream should answer with JSON")
}

#[tokio::test]
async fn a_request_is_forwarded_to_the_host_backend() {
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            matches:
              - path:
                  pathPrefix: /
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let body = get(&format!("{base}/some/path?a=b")).await;
    assert_eq!(body["upstream"], "only");
    assert_eq!(body["path"], "/some/path", "the path is preserved");
    assert_eq!(body["query"], "a=b", "the query string is preserved");
    assert_eq!(up.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn the_host_header_names_the_upstream_not_the_gateway() {
    // A name-based virtual host upstream serves the wrong site otherwise.
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let body = get(&format!("{base}/")).await;
    assert_eq!(
        body["headers"]["host"],
        format!("127.0.0.1:{}", up.port),
        "the upstream should see its own authority"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn the_client_address_is_recorded_in_the_forwarded_chain() {
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let body = get(&format!("{base}/")).await;
    assert_eq!(
        body["headers"]["x-forwarded-for"], "127.0.0.1",
        "the upstream needs the client address, not ours"
    );
    assert_eq!(body["headers"]["x-forwarded-proto"], "http");

    shutdown.cancel();
}

#[tokio::test]
async fn hop_by_hop_headers_do_not_reach_the_upstream() {
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let body: Value = reqwest::Client::new()
        .get(format!("{base}/"))
        .header("connection", "x-secret-hop")
        .header("x-secret-hop", "leaked")
        .send()
        .await
        .expect("request should reach the gateway")
        .json()
        .await
        .expect("should be JSON");

    assert!(
        body["headers"].get("x-secret-hop").is_none(),
        "a header named by Connection must not be forwarded: {}",
        body["headers"]
    );

    shutdown.cancel();
}

#[tokio::test]
async fn traffic_is_split_across_backends_by_weight() {
    let a = upstream("a").await;
    let b = upstream("b").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            backends:
              - host: "127.0.0.1:{}"
                weight: 1
              - host: "127.0.0.1:{}"
                weight: 3"#,
        a.port, b.port
    ))
    .await;

    for _ in 0..40 {
        get(&format!("{base}/")).await;
    }

    assert_eq!(
        (
            a.hits.load(Ordering::Relaxed),
            b.hits.load(Ordering::Relaxed)
        ),
        (10, 30),
        "a 1:3 split over 40 requests should land exactly"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_zero_weight_backend_receives_nothing() {
    let drained = upstream("drained").await;
    let live = upstream("live").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            backends:
              - host: "127.0.0.1:{}"
                weight: 0
              - host: "127.0.0.1:{}"
                weight: 1"#,
        drained.port, live.port
    ))
    .await;

    for _ in 0..10 {
        get(&format!("{base}/")).await;
    }

    assert_eq!(
        drained.hits.load(Ordering::Relaxed),
        0,
        "weight 0 is how a backend is drained without deleting its config"
    );
    assert_eq!(live.hits.load(Ordering::Relaxed), 10);

    shutdown.cancel();
}

#[tokio::test]
async fn a_prefix_rewrite_rewrites_only_the_matched_prefix() {
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            matches:
              - path:
                  pathPrefix: /api
            policies:
              urlRewrite:
                path:
                  prefix: /internal
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let body = get(&format!("{base}/api/v1/thing?keep=1")).await;
    assert_eq!(body["path"], "/internal/v1/thing");
    assert_eq!(body["query"], "keep=1", "a rewrite must not drop the query");

    shutdown.cancel();
}

#[tokio::test]
async fn header_modifiers_apply_in_both_directions() {
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              requestHeaderModifier:
                set:
                  x-gateway: rusty
                remove: [x-drop-me]
              responseHeaderModifier:
                set:
                  x-served-by: gateway
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let response = reqwest::Client::new()
        .get(format!("{base}/"))
        .header("x-drop-me", "should not arrive")
        .send()
        .await
        .expect("request should reach the gateway");

    assert_eq!(
        response
            .headers()
            .get("x-served-by")
            .and_then(|v| v.to_str().ok()),
        Some("gateway"),
        "response modifiers should apply on the way back"
    );

    let body: Value = response.json().await.expect("should be JSON");
    assert_eq!(body["headers"]["x-gateway"], "rusty");
    assert!(
        body["headers"].get("x-drop-me").is_none(),
        "a removed header must not reach the upstream"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_backend_key_replaces_the_clients_credential() {
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              backendAuth:
                key: backend-secret
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let body: Value = reqwest::Client::new()
        .get(format!("{base}/"))
        .header("authorization", "Bearer client-token")
        .send()
        .await
        .expect("request should reach the gateway")
        .json()
        .await
        .expect("should be JSON");

    assert_eq!(
        body["headers"]["authorization"], "Bearer backend-secret",
        "a client must not be able to smuggle its own credential upstream"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_unreachable_upstream_is_a_502() {
    // 502, not 500: the gateway is fine, the upstream is not, and conflating
    // the two sends people to debug the wrong process.
    let dead = free_port().await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            backends:
              - host: "127.0.0.1:{dead}""#
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("the gateway should still answer");
    assert_eq!(response.status(), 502);

    shutdown.cancel();
}

#[tokio::test]
async fn a_slow_upstream_hits_the_backend_timeout() {
    // Unlike MCP, a proxied response is not produced until the upstream
    // answers, so this budget genuinely bounds the wait.
    let port = free_port().await;
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("should bind");
    tokio::spawn(async move {
        let app = axum::Router::new().fallback(axum::routing::any(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            "too late"
        }));
        let _ = axum::serve(listener, app).await;
    });

    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              timeout:
                backendRequestTimeout: 300ms
            backends:
              - host: "127.0.0.1:{port}""#
    ))
    .await;

    let started = std::time::Instant::now();
    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("the gateway should answer");

    assert_eq!(response.status(), 504);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the budget should fire long before the 10s upstream; took {:?}",
        started.elapsed()
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_route_mixing_host_with_an_unsupported_kind_is_refused() {
    // Silently dropping the unsupported share onto the hosts would send
    // traffic somewhere the operator never asked for.
    let up = upstream("only").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: mixed
            backends:
              - host: "127.0.0.1:{}"
                weight: 1
              - dynamic: {{}}
                weight: 1"#,
        up.port
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("the gateway should answer");
    assert_eq!(response.status(), 501);
    assert_eq!(
        up.hits.load(Ordering::Relaxed),
        0,
        "no traffic should reach the host half of a mixed route"
    );

    shutdown.cancel();
}

/// An inventory naming `echo` backed by the given upstream ports.
fn inventory(ports: &[u16]) -> String {
    let mut yaml = String::from(
        "services:\n  - name: echo\n    namespace: default\n    ports:\n      80: 0\nworkloads:\n",
    );
    // The service port map is rewritten per workload below, so the service's
    // own target port is never the one used; `0` here would be a bug if it
    // ever were, which is the point of writing it down.
    for (index, port) in ports.iter().enumerate() {
        yaml.push_str(&format!(
            "  - name: echo-{index}\n    namespace: default\n    workloadIps: [\"127.0.0.1\"]\n    \
             services:\n      \"default/echo\":\n        80: {port}\n"
        ));
    }
    yaml
}

#[tokio::test]
async fn a_service_backend_reaches_the_workload_behind_it() {
    let up = upstream("service").await;
    let (base, shutdown) = start_with_inventory(
        r#"          - name: by-service
            backends:
              - service:
                  name: echo
                  port: 80"#,
        &inventory(&[up.port]),
    )
    .await;

    let answered = get(&format!("{base}/hello")).await;
    assert_eq!(answered["upstream"], "service");
    assert_eq!(answered["path"], "/hello");
    assert_eq!(up.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_service_spreads_traffic_across_its_workloads() {
    let one = upstream("one").await;
    let two = upstream("two").await;
    let (base, shutdown) = start_with_inventory(
        r#"          - name: by-service
            backends:
              - service:
                  name: echo
                  port: 80"#,
        &inventory(&[one.port, two.port]),
    )
    .await;

    for _ in 0..10 {
        let _ = get(&format!("{base}/")).await;
    }
    assert_eq!(one.hits.load(Ordering::Relaxed), 5);
    assert_eq!(two.hits.load(Ordering::Relaxed), 5);

    shutdown.cancel();
}

#[tokio::test]
async fn a_service_beside_a_host_gets_half_the_traffic_not_two_thirds() {
    // The reason the weights are scaled rather than repeated per workload.
    let one = upstream("service-one").await;
    let two = upstream("service-two").await;
    let literal = upstream("literal").await;

    let (base, shutdown) = start_with_inventory(
        &format!(
            r#"          - name: mixed
            backends:
              - service:
                  name: echo
                  port: 80
                weight: 1
              - host: "127.0.0.1:{}"
                weight: 1"#,
            literal.port
        ),
        &inventory(&[one.port, two.port]),
    )
    .await;

    for _ in 0..12 {
        let _ = get(&format!("{base}/")).await;
    }

    let service_hits = one.hits.load(Ordering::Relaxed) + two.hits.load(Ordering::Relaxed);
    assert_eq!(service_hits, 6, "half the route, split two ways");
    assert_eq!(literal.hits.load(Ordering::Relaxed), 6);

    shutdown.cancel();
}

#[tokio::test]
async fn an_unhealthy_workload_receives_nothing() {
    let good = upstream("good").await;
    let bad = upstream("bad").await;
    let (base, shutdown) = start_with_inventory(
        r#"          - name: by-service
            backends:
              - service:
                  name: echo
                  port: 80"#,
        &format!(
            "services:\n  - name: echo\n    namespace: default\nworkloads:\n\
             \x20 - name: good\n    workloadIps: [\"127.0.0.1\"]\n    services:\n      \
             \"default/echo\":\n        80: {}\n\
             \x20 - name: bad\n    workloadIps: [\"127.0.0.1\"]\n    status: unhealthy\n    \
             services:\n      \"default/echo\":\n        80: {}\n",
            good.port, bad.port
        ),
    )
    .await;

    for _ in 0..4 {
        let _ = get(&format!("{base}/")).await;
    }
    assert_eq!(good.hits.load(Ordering::Relaxed), 4);
    assert_eq!(bad.hits.load(Ordering::Relaxed), 0);

    shutdown.cancel();
}

#[tokio::test]
async fn a_service_the_inventory_does_not_hold_fails_at_startup() {
    // A route that could never serve a request is a typo in a file-driven
    // inventory, not a cluster in flux.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - backends:
              - service:
                  name: missing
                  port: 80
services:
  - name: echo
    namespace: default
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = common::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("an unresolvable service should not start");
    assert!(err.to_string().contains("missing"), "got: {err}");
    assert!(
        err.to_string().contains("default/echo"),
        "the error should say what the inventory does hold: {err}"
    );
}

#[tokio::test]
async fn a_dynamic_route_dials_the_address_the_client_asked_for() {
    // The plain forward-proxy reading: the request's own authority.
    let up = upstream("chosen").await;
    let (base, shutdown) = start(
        r#"          - name: forward
            backends:
              - dynamic: {}"#,
    )
    .await;

    // `Host` names the upstream; the gateway is reached by address.
    let answered: Value = reqwest::Client::new()
        .get(format!("{base}/hello"))
        .header("host", format!("127.0.0.1:{}", up.port))
        .send()
        .await
        .expect("the gateway should answer")
        .json()
        .await
        .expect("upstream should answer with JSON");

    assert_eq!(answered["upstream"], "chosen");
    assert_eq!(answered["path"], "/hello");
    assert_eq!(up.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_dynamic_target_expression_decides_instead() {
    // Where the address the client dialled is not the one to use: a trusted
    // hop's header picks the upstream.
    let asked = upstream("asked-for").await;
    let computed = upstream("computed").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: forward
            backends:
              - dynamic:
                  target: '"127.0.0.1:{}"'"#,
        computed.port
    ))
    .await;

    let answered: Value = reqwest::Client::new()
        .get(format!("{base}/"))
        .header("host", format!("127.0.0.1:{}", asked.port))
        .send()
        .await
        .expect("the gateway should answer")
        .json()
        .await
        .expect("upstream should answer with JSON");

    assert_eq!(answered["upstream"], "computed");
    assert_eq!(
        asked.hits.load(Ordering::Relaxed),
        0,
        "the expression decides, not the address the client dialled"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_dynamic_expression_reading_a_header_chooses_per_request() {
    let one = upstream("one").await;
    let two = upstream("two").await;
    let (base, shutdown) = start(
        r#"          - name: forward
            backends:
              - dynamic:
                  target: 'request.headers["x-upstream"]'"#,
    )
    .await;

    for (port, expected) in [(one.port, "one"), (two.port, "two")] {
        let answered: Value = reqwest::Client::new()
            .get(format!("{base}/"))
            .header("x-upstream", format!("127.0.0.1:{port}"))
            .send()
            .await
            .expect("the gateway should answer")
            .json()
            .await
            .expect("upstream should answer with JSON");
        assert_eq!(answered["upstream"], expected);
    }

    shutdown.cancel();
}

#[tokio::test]
async fn a_dynamic_request_naming_nowhere_is_refused() {
    let (base, shutdown) = start(
        r#"          - name: forward
            backends:
              - dynamic:
                  target: 'request.headers["x-upstream"]'"#,
    )
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("the gateway should answer");
    assert_eq!(response.status(), 400, "no guess is made");

    shutdown.cancel();
}

#[tokio::test]
async fn a_dynamic_backend_cannot_be_weighted_against_a_fixed_one() {
    // A share of "wherever the client asked for" is not a destination anyone
    // can reason about.
    let up = upstream("fixed").await;
    let (base, shutdown) = start(&format!(
        r#"          - name: mixed
            backends:
              - host: "127.0.0.1:{}"
                weight: 1
              - dynamic: {{}}
                weight: 1"#,
        up.port
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("the gateway should answer");
    assert_eq!(response.status(), 501);
    assert_eq!(up.hits.load(Ordering::Relaxed), 0);

    shutdown.cancel();
}

#[tokio::test]
async fn a_dynamic_target_that_does_not_compile_fails_at_startup() {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - backends:
              - dynamic:
                  target: "this is not cel"
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = common::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("an expression that cannot be compiled should not start");
    assert!(err.to_string().contains("CEL"), "got: {err}");
}

#[tokio::test]
async fn check_says_a_dynamic_route_lets_the_client_choose() {
    // Worth saying to anyone running `--check`, not only in the startup log of
    // a gateway already serving.
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - backends:
              - dynamic: {}
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("client chooses the upstream")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.contains("the address the client dialled")),
        "{findings:?}"
    );
}

/// Reads one request head from `stream`, returning its bytes and its
/// `Content-Length` body; `None` once the client hangs up.
async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<(String, Vec<u8>)> {
    use tokio::io::AsyncReadExt as _;
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        seen.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&seen[..head_end]).to_string();
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0);
    while seen.len() < head_end + length {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        seen.extend_from_slice(&buf[..n]);
    }
    Some((head, seen[head_end..head_end + length].to_vec()))
}

/// What a [`raw_upstream`] received: each request's head and body.
type Received = Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// A keep-alive upstream that answers every request with `body`, counting
/// the connections it accepts and recording the requests it receives.
async fn raw_upstream(body: Vec<u8>) -> (u16, Arc<AtomicUsize>, Received) {
    use tokio::io::AsyncWriteExt as _;
    let port = free_port().await;
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("upstream should bind");
    let accepted = Arc::new(AtomicUsize::new(0));
    let received: Received = Arc::default();
    let (counter, recorder) = (Arc::clone(&accepted), Arc::clone(&received));
    let body = Arc::new(body);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let (recorder, body) = (Arc::clone(&recorder), Arc::clone(&body));
            tokio::spawn(async move {
                while let Some(request) = read_request(&mut stream).await {
                    recorder.lock().expect("lock").push(request);
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                    if stream.write_all(head.as_bytes()).await.is_err()
                        || stream.write_all(&body).await.is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    (port, accepted, received)
}

/// The bodies of the requests `received` holds, in order.
fn bodies(received: &Received) -> Vec<Vec<u8>> {
    let received = received.lock().expect("lock");
    received.iter().map(|(_, body)| body.clone()).collect()
}

/// The values of every `name` field in `head`, in order.
fn header_values(head: &str, name: &str) -> Vec<String> {
    head.lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_string())
        .collect()
}

fn route_to(port: u16) -> String {
    format!(
        r#"          - name: proxy
            backends:
              - host: "127.0.0.1:{port}""#
    )
}

#[tokio::test]
async fn upstream_connections_are_reused_across_requests() {
    let (port, accepted, _) = raw_upstream(b"ok".to_vec()).await;
    let (base, shutdown) = start(&route_to(port)).await;
    let client = reqwest::Client::new();
    for _ in 0..3 {
        let response = client
            .get(format!("{base}/"))
            .send()
            .await
            .expect("should answer");
        assert_eq!(response.text().await.expect("body"), "ok");
    }
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "one upstream connection"
    );
    shutdown.cancel();
}

#[tokio::test]
async fn a_large_response_streams_through_intact() {
    let big: Vec<u8> = (0..4 * 1024 * 1024).map(|i: u32| (i % 251) as u8).collect();
    let (port, _, _) = raw_upstream(big.clone()).await;
    let (base, shutdown) = start(&route_to(port)).await;
    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(
        response.bytes().await.expect("body").as_ref(),
        big.as_slice()
    );
    shutdown.cancel();
}

#[tokio::test]
async fn a_body_over_max_request_body_bytes_is_refused() {
    let (port, _, received) = raw_upstream(b"ok".to_vec()).await;
    let (base, shutdown) = start_with_inventory(
        &route_to(port),
        "config:\n  limits:\n    maxRequestBodyBytes: 16",
    )
    .await;
    let client = reqwest::Client::new();
    let at_cap = client
        .post(format!("{base}/"))
        .body("x".repeat(16))
        .send()
        .await
        .expect("should answer");
    assert_eq!(at_cap.status(), 200);
    let over = client
        .post(format!("{base}/"))
        .body("x".repeat(17))
        .send()
        .await
        .expect("should answer");
    assert_eq!(over.status(), 413);
    assert_eq!(
        bodies(&received),
        vec![b"x".repeat(16)],
        "only the body within the cap reached the upstream"
    );
    shutdown.cancel();
}

#[tokio::test]
async fn a_chunked_request_body_is_forwarded_whole() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let (port, _, received) = raw_upstream(b"ok".to_vec()).await;
    let (base, shutdown) = start(&route_to(port)).await;
    let gateway = base.trim_start_matches("http://");
    let mut client = tokio::net::TcpStream::connect(gateway)
        .await
        .expect("connect");
    client
        .write_all(
            b"POST / HTTP/1.1\r\nHost: gateway\r\nTransfer-Encoding: chunked\r\n\
              Connection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
        )
        .await
        .expect("write");
    let mut answer = String::new();
    client.read_to_string(&mut answer).await.expect("read");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert_eq!(bodies(&received), vec![b"hello world".to_vec()]);
    shutdown.cancel();
}

#[tokio::test]
async fn request_headers_reach_the_upstream_as_the_client_sent_them() {
    let (port, _, received) = raw_upstream(b"ok".to_vec()).await;
    let (base, shutdown) = start(&route_to(port)).await;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("user-agent", "caller/1".parse().expect("value"));
    headers.append("x-multi", "a".parse().expect("value"));
    headers.append("x-multi", "b".parse().expect("value"));
    let client = reqwest::Client::new();
    client
        .get(format!("{base}/"))
        .headers(headers)
        .send()
        .await
        .expect("should answer");
    client
        .get(format!("{base}/"))
        .send()
        .await
        .expect("should answer");

    let received = received.lock().expect("lock");
    let (with_agent, without_agent) = (&received[0].0, &received[1].0);
    assert_eq!(
        header_values(with_agent, "user-agent"),
        vec!["caller/1"],
        "the client's User-Agent replaces the library's"
    );
    assert_eq!(header_values(with_agent, "x-multi"), vec!["a", "b"]);
    let agent = header_values(without_agent, "user-agent");
    assert!(
        agent.len() == 1 && agent[0].starts_with("rusty_request/"),
        "a request without one goes up with rusty_request's: {agent:?}"
    );
    shutdown.cancel();
}
