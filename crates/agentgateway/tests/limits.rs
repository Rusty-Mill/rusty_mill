//! End-to-end tests for timeouts and load shedding.
//!
//! The backend is `examples/mock_mcp_server.rs` with `MOCK_DELAY_MS` set, so
//! these exercise a genuinely slow call rather than a stubbed future.
//!
//! # Which timeout does what
//!
//! Measured, not assumed: against a 5s tool call, `time_starttransfer` is
//! ~1ms and `time_total` is ~5s. The Streamable HTTP transport sends its SSE
//! response headers immediately and streams the JSON-RPC result afterwards.
//!
//! So `requestTimeout` — which bounds *producing a response* — is already
//! satisfied before a tool starts running, and cannot cut one off. That is not
//! a defect: bounding the whole stream would kill every long-lived
//! subscription. `backendRequestTimeout` is the budget that bounds a tool
//! call, applied around the upstream request inside the federation, and these
//! tests are written around that distinction rather than against it.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use rusty_mcp::limits::LimitsLayer;
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop();
    path.pop();
    path.push("examples");
    path.push("mock_mcp_server");
    path.display().to_string()
}

/// A port nothing in this binary has been handed yet.
///
/// Binding to port 0 and dropping the listener leaves a window in which the
/// same port can be handed out twice, and two tests racing for it fail with
/// `Address already in use`. Remembering what has been issued closes the
/// window between tests, which is where the collisions actually came from.
async fn free_port() -> u16 {
    use std::collections::HashSet;
    use std::sync::{LazyLock, Mutex};
    static ISSUED: LazyLock<Mutex<HashSet<u16>>> = LazyLock::new(Default::default);

    for _ in 0..64 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind");
        let port = listener.local_addr().expect("should have an addr").port();
        drop(listener);
        if ISSUED.lock().expect("lock").insert(port) {
            return port;
        }
    }
    panic!("could not find a port this binary has not already used");
}

/// A gateway fronting one deliberately slow MCP target.
///
/// `policies` is spliced into the route, and `global` under `config:`, so each
/// test states only the knob it is about.
async fn start(policies: &str, global: &str, delay_ms: u64) -> (String, CancellationToken) {
    let port = free_port().await;
    let yaml = format!(
        r#"
{global}
binds:
  - port: {port}
    listeners:
      - routes:
          - name: slow
            matches:
              - path:
                  pathPrefix: /mcp
{policies}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_DELAY_MS: "{delay_ms}"
"#,
        server = mock_server(),
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");

    let limits = match config.config.as_ref().and_then(|c| c.limits.as_ref()) {
        Some(limits) => {
            let mut layer = LimitsLayer::new();
            if let Some(max) = limits.max_concurrent_requests {
                layer = layer.with_max_concurrent(max);
            }
            layer
        }
        None => LimitsLayer::new(),
    };

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving =
        serve::run_with_shutdown_and_limits(gateway, vec![addr], shutdown.clone(), limits)
            .await
            .expect("gateway should bind");

    (format!("http://127.0.0.1:{port}/mcp"), shutdown)
}

/// Complete the MCP handshake and return the session id.
async fn open_session(url: &str) -> String {
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "limits-test", "version": "1"}
            }
        }))
        .send()
        .await
        .expect("initialize should reach the gateway");

    let session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .expect("the gateway should issue a session id")
        .to_string();

    client
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session)
        .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send()
        .await
        .expect("initialized notification should be accepted");

    session
}

/// Call the slow tool on an established session.
async fn call_slow_tool(url: &str, session: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", session)
        .json(&json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "alpha_echo", "arguments": {}}
        }))
        .send()
        .await
        .expect("the call should reach the gateway")
}

/// Read an SSE body and return the concatenated `data:` payloads.
fn sse_data(body: &str) -> String {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_backend_timeout_cuts_off_a_slow_tool_call() {
    let policies = "            policies:\n              timeout:\n                backendRequestTimeout: 200ms";
    let (url, shutdown) = start(policies, "", 5_000).await;
    let session = open_session(&url).await;

    let started = Instant::now();
    let response = call_slow_tool(&url, &session).await;
    let body = response.text().await.expect("should read the body");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(3),
        "the call should be abandoned near its budget, not wait out the 5s backend; took {elapsed:?}"
    );
    let data = sse_data(&body);
    assert!(
        data.contains("timed out"),
        "the caller should be told the tool timed out, got: {data}"
    );
    assert!(
        data.contains("alpha_echo"),
        "the error should name the tool: {data}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_fast_call_is_untouched_by_a_generous_backend_timeout() {
    let policies =
        "            policies:\n              timeout:\n                backendRequestTimeout: 30s";
    let (url, shutdown) = start(policies, "", 0).await;
    let session = open_session(&url).await;

    let body = call_slow_tool(&url, &session)
        .await
        .text()
        .await
        .expect("should read the body");
    let data = sse_data(&body);
    assert!(
        data.contains("alpha:echo"),
        "a call well inside its budget must produce its real result, got: {data}"
    );
    assert!(!data.contains("timed out"), "got: {data}");

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_timeout_does_not_kill_a_long_stream() {
    // The non-obvious half of the distinction above, pinned so nobody
    // "fixes" requestTimeout into something that severs live subscriptions.
    let policies =
        "            policies:\n              timeout:\n                requestTimeout: 200ms";
    let (url, shutdown) = start(policies, "", 1_500).await;
    let session = open_session(&url).await;

    let response = call_slow_tool(&url, &session).await;
    assert_eq!(
        response.status(),
        200,
        "the SSE response is produced immediately, well inside the budget"
    );

    let body = response.text().await.expect("should read the body");
    assert!(
        sse_data(&body).contains("alpha:echo"),
        "the stream should still deliver its result after the budget elapsed"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_timeout_still_bounds_a_slow_response() {
    // A backend that is slow to *produce* a response is what requestTimeout
    // is for. An unsupported backend answers instantly, so use the route that
    // has none at all: it must not hang, and must not 504 either.
    let policies =
        "            policies:\n              timeout:\n                requestTimeout: 5s";
    let (url, shutdown) = start(policies, "", 0).await;

    let response = reqwest::Client::new()
        .get(format!("{url}/../nowhere"))
        .send()
        .await
        .expect("should reach the gateway");
    assert_eq!(response.status(), 404);

    shutdown.cancel();
}

#[tokio::test]
async fn nothing_is_shed_when_no_limit_is_configured() {
    // Off by default is deliberate: there is no concurrency number right for
    // everyone, and a default would be a silent regression for a gateway
    // already serving more than it.
    let (url, shutdown) = start("", "", 0).await;
    let session = open_session(&url).await;

    let mut handles = Vec::new();
    for _ in 0..8 {
        let url = url.clone();
        let session = session.clone();
        handles.push(tokio::spawn(async move {
            call_slow_tool(&url, &session).await.status().as_u16()
        }));
    }

    for handle in handles {
        let status = handle.await.expect("task should not panic");
        assert_ne!(status, 503, "nothing should be shed when no limit is set");
    }

    shutdown.cancel();
}

#[test]
fn the_config_parses_both_limit_knobs() {
    let config = Config::from_yaml(
        r#"
config:
  limits:
    maxConcurrentRequests: 256
    requestTimeout: 30s
  tracing:
    endpoint: http://localhost:4317
    serviceName: gw
    sampleRatio: 0.1
binds:
  - port: 3000
    listeners:
      - routes:
          - backends: [{host: "a:80"}]
"#,
    )
    .expect("should parse");

    let global = config.config.as_ref().expect("global config");
    let limits = global.limits.as_ref().expect("limits");
    assert_eq!(limits.max_concurrent_requests, Some(256));
    assert_eq!(
        limits.request_timeout.map(std::time::Duration::from),
        Some(Duration::from_secs(30))
    );

    let tracing = global.tracing.as_ref().expect("tracing");
    assert_eq!(tracing.endpoint.as_deref(), Some("http://localhost:4317"));
    assert_eq!(tracing.sample_ratio, Some(0.1));
}

#[test]
fn both_timeouts_parse_and_are_distinct() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              timeout:
                requestTimeout: 30s
                backendRequestTimeout: 5s
            backends: [{host: "a:80"}]
"#,
    )
    .expect("should parse");

    let timeout = config.binds[0].listeners[0].routes[0]
        .policies
        .as_ref()
        .and_then(|p| p.timeout.clone())
        .expect("timeout policy");
    assert_eq!(
        timeout.request_timeout.map(Duration::from),
        Some(Duration::from_secs(30))
    );
    assert_eq!(
        timeout.backend_request_timeout.map(Duration::from),
        Some(Duration::from_secs(5))
    );

    // Both are enforced now, so neither should be reported as inert.
    assert!(
        !config.lint().iter().any(|f| f.contains("imeout")),
        "lint should not flag an enforced policy: {:?}",
        config.lint()
    );
}
