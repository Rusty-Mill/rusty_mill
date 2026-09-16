//! Covers spec Sections 3.1.1/3.1.2's `UnsupportedOperationError`
//! requirement: "Messages sent to Tasks that are in a terminal state
//! (COMPLETED, FAILED, CANCELED, REJECTED) cannot accept further
//! messages." `Engine::start_execution` is the single choke point both
//! `SendMessage` and `SendStreamingMessage` go through, so one guard
//! there covers every binding.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskState};

/// Completes immediately, so every task this agent creates reaches a
/// terminal state on its very first turn.
struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Terminal Task Rejection Test Agent",
        "An A2A agent used for rusty_a2a's terminal-task rejection tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_streaming(true);

    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

fn continuation(task_id: &str) -> Message {
    Message::user_text("are you still there?").with_task_id(task_id)
}

#[tokio::test]
async fn send_message_to_a_completed_task_is_rejected() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    assert_eq!(task.status.state, TaskState::Completed);

    let err = client
        .send_message(continuation(&task.id), None)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::UnsupportedOperation(_)) => {}
        other => panic!("expected UnsupportedOperation, got {other:?}"),
    }
}

#[tokio::test]
async fn send_streaming_message_to_a_completed_task_is_rejected() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let err = client
        .send_streaming_message(continuation(&task_id), None)
        .await
        .map(|_| ())
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::UnsupportedOperation(_)) => {}
        other => panic!("expected UnsupportedOperation, got {other:?}"),
    }
}

#[tokio::test]
async fn rest_binding_rejects_the_same_way() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let sent: serde_json::Value = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send")
        .json()
        .await
        .expect("response body");
    assert_eq!(sent["task"]["status"]["state"], "TASK_STATE_COMPLETED");
    let task_id = sent["task"]["id"].as_str().expect("task id").to_string();

    let resp = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {
                "messageId": "m2",
                "taskId": task_id,
                "role": "ROLE_USER",
                "parts": [{"text": "are you still there?"}]
            }
        }))
        .send()
        .await
        .expect("POST /message:send (continuation)");
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["error"]["status"], "FAILED_PRECONDITION");
}
