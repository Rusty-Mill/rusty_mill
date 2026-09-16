//! Covers a handful of input-validation gaps found in the third compliance
//! audit:
//! - `pageSize`/`historyLength` out of range (spec Section 3.1.4 / 3.3.2)
//!   are now a validation error, not silently clamped/coerced.
//! - An `Artifact` with no `parts` (spec Section 4.1.7) never reaches the
//!   wire.
//! - The JSON-RPC binding distinguishes `-32700` (invalid JSON) from
//!   `-32600` (syntactically valid JSON that isn't a valid Request object)
//!   instead of collapsing both into `-32700` (spec Section 9.5).
//! - The REST binding's own extractor rejections (malformed JSON body, an
//!   unparsable query parameter) still use the `google.rpc.Status` JSON
//!   envelope (spec Section 11.6), instead of `axum`'s default plain-text
//!   rejection response.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Artifact, ListTasksRequest, Message, Part, TaskState};

struct TestAgent;

#[async_trait]
impl AgentExecutor for TestAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        if ctx.message.text().contains("empty artifact") {
            events.artifact(Artifact::new("empty", Vec::new()));
            events.artifact(Artifact::new("real", vec![Part::text("content")]));
            events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
            return Ok(());
        }
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Input Validation Test Agent",
        "An A2A agent used for rusty_a2a's input validation tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    let server = AgentServer::new(card, Arc::new(TestAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

fn expect_invalid_params(err: ClientError) {
    match err {
        ClientError::Protocol(A2aError::InvalidParams(_)) => {}
        other => panic!("expected InvalidParams, got {other:?}"),
    }
}

#[tokio::test]
async fn page_size_out_of_range_is_rejected() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    for page_size in [0, -1, 101, 1000] {
        let err = client
            .list_tasks(ListTasksRequest {
                page_size: Some(page_size),
                ..Default::default()
            })
            .await
            .unwrap_err();
        expect_invalid_params(err);
    }

    // In-range values are still accepted.
    client
        .list_tasks(ListTasksRequest {
            page_size: Some(1),
            ..Default::default()
        })
        .await
        .expect("pageSize=1 should be accepted");
    client
        .list_tasks(ListTasksRequest {
            page_size: Some(100),
            ..Default::default()
        })
        .await
        .expect("pageSize=100 should be accepted");
}

#[tokio::test]
async fn negative_history_length_is_rejected() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let err = client.get_task(&task_id, Some(-1)).await.unwrap_err();
    expect_invalid_params(err);

    // 0 is valid (means "no history"), not an error.
    client
        .get_task(&task_id, Some(0))
        .await
        .expect("historyLength=0 should be accepted");
}

#[tokio::test]
async fn an_artifact_with_no_parts_never_reaches_the_wire() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please send an empty artifact"), None)
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    assert!(
        !task.artifacts.iter().any(|a| a.artifact_id == "empty"),
        "an artifact with no parts must never reach the wire, got {:?}",
        task.artifacts
    );
    assert!(
        task.artifacts.iter().any(|a| a.artifact_id == "real"),
        "the artifact with real parts must still go through, got {:?}",
        task.artifacts
    );
}

#[tokio::test]
async fn malformed_json_gets_json_rpc_parse_error() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{base_url}/"))
        .header("content-type", "application/json")
        .body("{not valid json")
        .send()
        .await
        .expect("POST /");
    let body: serde_json::Value = resp.json().await.expect("JSON-RPC error body");
    assert_eq!(body["error"]["code"], -32700);
}

#[tokio::test]
async fn syntactically_valid_json_that_is_not_a_request_gets_invalid_request() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    // Valid JSON, but not a JSON-RPC Request object (missing `method`).
    let resp = http
        .post(format!("{base_url}/"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1}))
        .send()
        .await
        .expect("POST /");
    let body: serde_json::Value = resp.json().await.expect("JSON-RPC error body");
    assert_eq!(body["error"]["code"], -32600);
}

#[tokio::test]
async fn rest_malformed_json_body_uses_the_google_rpc_status_envelope() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{base_url}/message:send"))
        .header("content-type", "application/json")
        .body("{not valid json")
        .send()
        .await
        .expect("POST /message:send");
    assert_eq!(resp.status(), 400);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.contains("application/a2a+json"),
        "expected the a2a+json envelope content type, got {content_type:?}"
    );
    let body: serde_json::Value = resp.json().await.expect("google.rpc.Status body");
    assert!(body["error"]["code"].is_number());
    assert!(body["error"]["status"].is_string());
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn rest_invalid_query_parameter_uses_the_google_rpc_status_envelope() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .get(format!("{base_url}/tasks?pageSize=notanumber"))
        .send()
        .await
        .expect("GET /tasks");
    assert_eq!(resp.status(), 400);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.contains("application/a2a+json"),
        "expected the a2a+json envelope content type, got {content_type:?}"
    );
    let body: serde_json::Value = resp.json().await.expect("google.rpc.Status body");
    assert!(body["error"]["code"].is_number());
    assert!(body["error"]["status"].is_string());
    assert!(body["error"]["message"].is_string());
}
