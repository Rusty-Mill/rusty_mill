//! End-to-end tests for the `agui` policy.
//!
//! The backend is a mock AG-UI agent that records what reached it and answers
//! with a canned event stream, so the assertions are about what the agent
//! saw (or never saw) and what a client reads back through the gateway.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

mod common;
use common::free_port;

const STREAM: &str = "data: {\"type\":\"RUN_STARTED\",\"threadId\":\"t\",\"runId\":\"r\"}\n\n\
data: {\"type\":\"TEXT_MESSAGE_CHUNK\",\"messageId\":\"m\",\"delta\":\"hi\"}\n\n\
data: {\"type\":\"RUN_FINISHED\",\"threadId\":\"t\",\"runId\":\"r\"}\n\n";

struct Agent {
    port: u16,
    hits: Arc<AtomicUsize>,
    last: Arc<Mutex<(String, Value)>>,
}

async fn agent() -> Agent {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let hits = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(Mutex::new((String::new(), Value::Null)));
    let counter = Arc::clone(&hits);
    let recorder = Arc::clone(&last);

    let app = Router::new().fallback(any(move |request: Request| {
        let counter = Arc::clone(&counter);
        let recorder = Arc::clone(&recorder);
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            let method = request.method().to_string();
            let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                .await
                .unwrap_or_default();
            let body = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            if let Ok(mut seen) = recorder.lock() {
                *seen = (method, body);
            }
            axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(axum::body::Body::from(STREAM))
                .expect("response should build")
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("agent should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Agent { port, hits, last }
}

/// Boot a gateway with an `agui` route carrying `rules` in front of `agent`.
async fn start(rules: &[&str], agent: u16) -> (String, CancellationToken) {
    let port = free_port().await;
    let rules: String = rules
        .iter()
        .map(|line| format!("                  {line}\n"))
        .collect();
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: assistant
            policies:
              agui:
                rules:
{rules}
            backends:
              - host: "127.0.0.1:{agent}"
"#
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

    (format!("http://127.0.0.1:{port}"), shutdown)
}

fn run_input(tools: &[&str]) -> Value {
    json!({
        "threadId": "t",
        "runId": "r",
        "messages": [{"id": "u1", "role": "user", "content": "add buy milk"}],
        "tools": tools.iter().map(|name| json!({
            "name": name, "description": "", "parameters": {"type": "object"}
        })).collect::<Vec<_>>(),
        "context": [],
        "state": {},
        "forwardedProps": {}
    })
}

async fn run(base: &str, input: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/api/agent"))
        .json(input)
        .send()
        .await
        .expect("request should reach the gateway")
}

#[tokio::test]
async fn no_allow_rule_refuses_every_run() {
    let a = agent().await;
    let (base, shutdown) = start(&[], a.port).await;

    let response = run(&base, &run_input(&[])).await;
    assert_eq!(response.status(), 403);
    assert!(
        response
            .text()
            .await
            .expect("text")
            .contains("no allow rule"),
        "the reason names the gap"
    );
    assert_eq!(a.hits.load(Ordering::Relaxed), 0, "the agent never saw it");

    shutdown.cancel();
}

#[tokio::test]
async fn an_allowed_run_streams_through_untouched() {
    let a = agent().await;
    let (base, shutdown) = start(
        &["- allow: 'agui.lastUserMessage.startsWith(\"add\")'"],
        a.port,
    )
    .await;

    let response = run(&base, &run_input(&["create_task"])).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
    assert_eq!(response.text().await.expect("text"), STREAM);
    assert_eq!(a.hits.load(Ordering::Relaxed), 1);
    let (method, body) = a.last.lock().expect("lock").clone();
    assert_eq!(method, "POST");
    assert_eq!(
        body["tools"][0]["name"], "create_task",
        "the body arrives intact"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn deny_and_require_narrow_an_allow() {
    let a = agent().await;
    let (base, shutdown) = start(
        &[
            "- allow: 'true'",
            "- deny: '\"delete_task\" in agui.tools'",
            "- require: 'agui.tools.size() > 0'",
        ],
        a.port,
    )
    .await;

    let denied = run(&base, &run_input(&["delete_task"])).await;
    assert_eq!(denied.status(), 403);
    assert!(
        denied
            .text()
            .await
            .expect("text")
            .contains("deny rule matched")
    );

    let unmet = run(&base, &run_input(&[])).await;
    assert_eq!(unmet.status(), 403);
    assert!(
        unmet
            .text()
            .await
            .expect("text")
            .contains("require rule failed")
    );

    assert_eq!(a.hits.load(Ordering::Relaxed), 0);
    assert_eq!(run(&base, &run_input(&["create_task"])).await.status(), 200);
    assert_eq!(a.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_post_that_is_not_a_run_is_a_400_and_a_get_passes_through() {
    let a = agent().await;
    let (base, shutdown) = start(&["- allow: 'true'"], a.port).await;

    let response = run(&base, &json!({"hello": "world"})).await;
    assert_eq!(response.status(), 400);
    assert_eq!(a.hits.load(Ordering::Relaxed), 0);

    let get = reqwest::get(format!("{base}/api/agent/health"))
        .await
        .expect("request should reach the gateway");
    assert_eq!(get.status(), 200);
    assert_eq!(
        a.hits.load(Ordering::Relaxed),
        1,
        "not a run: proxied as is"
    );
    assert_eq!(a.last.lock().expect("lock").0, "GET");

    shutdown.cancel();
}

#[test]
fn a_route_cannot_carry_both_a2a_and_agui() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - policies:
              a2a: {}
              agui: {}
            backends:
              - host: "127.0.0.1:1"
"#,
    )
    .expect("config should parse");
    let err = config.validate().expect_err("should be refused");
    assert!(err.to_string().contains("either `a2a` or `agui`"), "{err}");
}
