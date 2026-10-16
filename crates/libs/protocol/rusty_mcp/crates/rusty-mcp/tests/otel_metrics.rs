//! OTLP metric export.
//!
//! The unit tests cover which labels are chosen and how they are encoded. What
//! only shows up end to end is whether they leave the process at all, in the
//! right shape, over HTTP — and, more importantly, whether the cardinality
//! guard holds on the wire. A label the layer decided to drop but the exporter
//! sent anyway would be worth nothing.

#![cfg(feature = "otel")]

mod support;

use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::response::Response;
use http::Request;
use rusty_json::Value;
use rusty_mcp::otel::{
    OtelConfig, OtelGuard,
    metrics::{McpMetricsLayer, Outcome, TaskOutcome},
};
use tower_layer::Layer as _;
use tower_service::Service as _;

/// One request the fake collector received.
#[derive(Debug)]
struct Push {
    path: String,
    content_type: String,
    body: Value,
}

type Received = Arc<Mutex<Vec<Push>>>;

/// A fake OTLP/HTTP collector: records what is POSTed, answers `status`.
///
/// The body is parsed as JSON, so the tests can look at real structure rather
/// than scan bytes for a string.
async fn spawn_collector(received: Received, status: u16) -> SocketAddr {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind collector");
    let addr = listener.local_addr().expect("addr");

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let received = Arc::clone(&received);

            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = vec![0u8; 16 * 1024];
                // Read until the head and the whole `Content-Length` body are in.
                loop {
                    let Ok(n) = socket.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if request_is_complete(&raw) {
                        break;
                    }
                }

                if let Some(push) = parse_request(&raw) {
                    received.lock().expect("lock").push(push);
                }
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    addr
}

fn head_and_body(raw: &[u8]) -> Option<(String, &[u8])> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    Some((
        String::from_utf8_lossy(&raw[..end]).into_owned(),
        &raw[end + 4..],
    ))
}

fn header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn request_is_complete(raw: &[u8]) -> bool {
    let Some((head, body)) = head_and_body(raw) else {
        return false;
    };
    let length: usize = header(&head, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    body.len() >= length
}

fn parse_request(raw: &[u8]) -> Option<Push> {
    let (head, body) = head_and_body(raw)?;
    let request_line = head.lines().next()?;
    assert!(request_line.starts_with("POST "), "{request_line}");
    Some(Push {
        path: request_line.split_whitespace().nth(1)?.to_owned(),
        content_type: header(&head, "content-type").unwrap_or_default(),
        body: Value::parse(std::str::from_utf8(body).ok()?).ok()?,
    })
}

fn pipeline_to(addr: SocketAddr) -> OtelGuard {
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_metrics_endpoint(format!("http://{addr}/v1/metrics"))
            // Long enough that nothing is pushed on a timer: every assertion
            // below is about what the explicit flush or shutdown sends.
            .with_metrics_interval(Duration::from_secs(3600))
            .with_shutdown_timeout(Duration::from_secs(2)),
    )
    .expect("pipeline starts");

    guard
}

/// The most recent push's metrics, by name.
fn metric(received: &Received, name: &str) -> Option<Value> {
    let received = received.lock().expect("lock");
    let push = received.last()?;
    push.body
        .get("resourceMetrics")?
        .get_index(0)?
        .get("scopeMetrics")?
        .get_index(0)?
        .get("metrics")?
        .as_array()?
        .iter()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(name))
        .cloned()
}

/// Every attribute value of every data point of every metric in the last push.
fn attribute_values(received: &Received) -> Vec<String> {
    let received = received.lock().expect("lock");
    let Some(push) = received.last() else {
        return Vec::new();
    };
    let text = push.body.to_json_string();
    let Ok(parsed) = Value::parse(&text) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    collect_string_values(&parsed, &mut values);
    values
}

/// Every string anywhere in `value`.
fn collect_string_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| collect_string_values(v, out)),
        Value::Object(map) => map.iter().for_each(|(_, v)| collect_string_values(v, out)),
        _ => {}
    }
}

fn saw(received: &Received, needle: &str) -> bool {
    attribute_values(received).iter().any(|v| v == needle)
}

/// The `asInt` of the first data point of a sum metric.
fn as_int(metric: &Value) -> Option<String> {
    metric
        .get("sum")?
        .get("dataPoints")?
        .get_index(0)?
        .get("asInt")?
        .as_str()
        .map(str::to_owned)
}

fn pushes(received: &Received) -> usize {
    received.lock().expect("lock").len()
}

/// A service that answers every request with `status`.
#[derive(Clone)]
struct Fixed(http::StatusCode);

impl<B> tower_service::Service<Request<B>> for Fixed {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request<B>) -> Self::Future {
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = self.0;
        std::future::ready(Ok(response))
    }
}

/// Drive one request through the layer.
async fn call_through(layer: &McpMetricsLayer, status: http::StatusCode, headers: &[(&str, &str)]) {
    let mut service = layer.layer(Fixed(status));

    let mut builder = Request::builder();
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(axum::body::Body::empty()).expect("request");

    let _ = service.call(request).await.expect("infallible");
}

#[tokio::test(flavor = "multi_thread")]
async fn instruments_reach_the_collector_after_a_flush() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let instruments = guard.instruments().expect("metrics are on by default");

    instruments.request_started("tools/call");
    instruments.request_finished("tools/call", Some("add"), Outcome::Ok, 0.012);

    // The flush is what sends them; nothing is pushed on a timer here.
    guard.shutdown();

    assert_eq!(pushes(&received), 1);
    {
        let received = received.lock().expect("lock");
        assert_eq!(received[0].path, "/v1/metrics");
        assert_eq!(received[0].content_type, "application/json");
    }
    assert!(
        metric(&received, "mcp.server.requests").is_some(),
        "the request counter never reached the collector"
    );
    assert!(
        metric(&received, "mcp.server.request.duration").is_some(),
        "the duration histogram never reached the collector"
    );
    assert!(saw(&received, "test-server"), "service.name is missing");
    assert!(saw(&received, "rusty-mcp"), "the scope name is missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_layer_records_a_request_with_its_tool_name() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ))
    .with_known_names(["add"]);

    call_through(
        &layer,
        http::StatusCode::OK,
        &[("mcp-method", "tools/call"), ("mcp-name", "add")],
    )
    .await;

    guard.shutdown();

    assert!(metric(&received, "mcp.server.requests").is_some());
    assert!(saw(&received, "tools/call"), "the method label is missing");
    assert!(saw(&received, "add"), "the tool name label is missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tool_name_never_reaches_the_collector() {
    // The cardinality guarantee, proven on the wire rather than in a unit test.
    // Anyone who can reach the endpoint can call a tool that does not exist; if
    // that name became a label, they would own the label space.
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ))
    .with_known_names(["add"]);

    for i in 0..20 {
        call_through(
            &layer,
            http::StatusCode::OK,
            &[
                ("mcp-method", "tools/call"),
                ("mcp-name", &format!("forged-tool-{i:02}")),
            ],
        )
        .await;
    }

    guard.shutdown();

    // Counted, but under one `other` series, not twenty.
    let counter = metric(&received, "mcp.server.requests").expect("counted");
    let series = counter
        .get("sum")
        .and_then(|s| s.get("dataPoints"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    assert_eq!(series, 1, "forged names fanned out into {series} series");
    assert_eq!(as_int(&counter).as_deref(), Some("20"));
    assert!(
        !attribute_values(&received)
            .iter()
            .any(|v| v.contains("forged-tool-")),
        "a client-supplied tool name was exported as a label"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_method_never_reaches_the_collector() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    call_through(
        &layer,
        http::StatusCode::OK,
        &[("mcp-method", "a-method-invented-by-a-client")],
    )
    .await;

    guard.shutdown();

    assert!(
        !attribute_values(&received)
            .iter()
            .any(|v| v.contains("a-method-invented-by-a-client")),
        "an unknown method was exported as a label"
    );
    assert!(saw(&received, "other"), "it should be bucketed as `other`");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_request_is_still_counted() {
    // The layer sits outside authorization precisely so this is true. A flood
    // of rejected tokens must not look like no traffic at all.
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    call_through(
        &layer,
        http::StatusCode::UNAUTHORIZED,
        &[("mcp-method", "tools/call")],
    )
    .await;

    guard.shutdown();

    assert!(
        saw(&received, "unauthorized"),
        "a 401 should be counted with its own outcome"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn task_outcomes_are_counted() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let instruments = guard.instruments().expect("metrics are enabled");

    instruments.task_started();
    instruments.task_finished(TaskOutcome::Completed);
    instruments.tasks_abandoned(3);

    guard.shutdown();

    assert!(metric(&received, "mcp.server.tasks.started").is_some());
    let finished = metric(&received, "mcp.server.tasks.finished").expect("finished");
    assert!(saw(&received, "abandoned"), "the drain count is missing");
    let abandoned = finished
        .get("sum")
        .and_then(|s| s.get("dataPoints"))
        .and_then(Value::as_array)
        .and_then(|points| {
            points.iter().find(|p| {
                p.get("attributes")
                    .map(Value::to_json_string)
                    .is_some_and(|a| a.contains("abandoned"))
            })
        })
        .and_then(|p| p.get("asInt").and_then(Value::as_str).map(str::to_owned));
    assert_eq!(abandoned.as_deref(), Some("3"));
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_can_be_turned_off() {
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint("http://127.0.0.1:1")
            .without_metrics(),
    )
    .expect("pipeline starts");

    assert!(guard.instruments().is_none());

    // Neither shutdown nor flush may panic with no push thread behind them.
    guard.flush();
    guard.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_layer_counts_requests_to_a_mounted_server() {
    // The layer on a real mounted MCP server, not just driven directly.
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    let (server, _) = support::spawn(|mcp| mcp.layer(layer)).await;
    assert!(
        support::post(server, "tools/list", None, support::tools_list(), None)
            .await
            .status()
            .is_success()
    );

    guard.shutdown();

    assert!(metric(&received, "mcp.server.requests").is_some());
    assert!(saw(&received, "tools/list"), "the method label is missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_rejected_by_the_auth_layer_is_still_counted() {
    // The ordering claim, end to end: metrics mount *outside* authorization, so
    // a request that never reaches the handler is still counted. Mounted the
    // other way round, a flood of bad tokens would look like no traffic at all.
    use rusty_mcp::auth::{AuthConfig, RequireAuthLayer, StaticTokenValidator, VerifiedToken};

    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    let validator = StaticTokenValidator::new().with_token(
        "good-token",
        VerifiedToken::new(["https://mcp.example.com/mcp"]),
    );
    let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(validator))
        .expect("valid resource")
        .with_authorization_servers(["https://auth.example.com"]);

    // Metrics are the later (outer) layer, so they see what auth rejects.
    let (server, _) =
        support::spawn(|mcp| mcp.layer(RequireAuthLayer::new(auth)).layer(layer)).await;

    let status = support::post(server, "tools/list", None, support::tools_list(), None)
        .await
        .status();
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);

    guard.shutdown();

    assert!(
        saw(&received, "unauthorized"),
        "the rejected request was never counted, so metrics sit inside the guard"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn values_are_cumulative_across_pushes() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    let instruments = guard.instruments().expect("metrics are enabled");

    instruments.task_started();
    guard.flush();
    let first = metric(&received, "mcp.server.tasks.started").expect("first push");
    assert_eq!(as_int(&first).as_deref(), Some("1"));

    instruments.task_started();
    guard.flush();
    let second = metric(&received, "mcp.server.tasks.started").expect("second push");
    // The second push carries the total, not the delta, so a lost push loses
    // nothing.
    assert_eq!(as_int(&second).as_deref(), Some("2"));
    assert_eq!(pushes(&received), 2);

    guard.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_pushed_until_something_is_recorded() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let guard = pipeline_to(addr);
    guard.flush();
    guard.shutdown();

    assert_eq!(pushes(&received), 0, "an empty registry should stay quiet");
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_are_pushed_on_the_timer_without_a_flush() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 200).await;

    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_metrics_endpoint(format!("http://{addr}/v1/metrics"))
            .with_metrics_interval(Duration::from_millis(100)),
    )
    .expect("pipeline starts");
    guard.instruments().expect("on").task_started();

    let deadline = Instant::now() + Duration::from_secs(5);
    while pushes(&received) == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(pushes(&received) > 0, "the timer never pushed");

    guard.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_collector_that_rejects_the_push_does_not_break_anything() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received), 500).await;

    let guard = pipeline_to(addr);
    let instruments = guard.instruments().expect("metrics are enabled");
    instruments.task_started();

    // Each call pushes and is refused; neither may panic or hang.
    guard.flush();
    guard.shutdown();
    assert_eq!(pushes(&received), 2, "the flush and the final push");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_metrics_endpoint_that_is_not_http_is_refused_at_start() {
    let result = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint("http://127.0.0.1:1")
            .with_metrics_endpoint("localhost:4318/v1/metrics"),
    );
    let Err(err) = result else {
        panic!("a schemeless metrics endpoint should be refused");
    };
    assert!(
        err.to_string().contains("not a valid http(s) URL"),
        "unexpected error: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_collector_does_not_break_request_handling() {
    // Telemetry is not load-bearing.
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint("http://127.0.0.1:1")
            // Port 1 is reliably closed.
            .with_metrics_endpoint("http://127.0.0.1:1/v1/metrics")
            .with_shutdown_timeout(Duration::from_secs(2)),
    )
    .expect("the pipeline starts even if the collector is unreachable");

    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    call_through(
        &layer,
        http::StatusCode::OK,
        &[("mcp-method", "tools/list")],
    )
    .await;

    guard.shutdown();
}
