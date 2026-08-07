//! Covers two smaller compliance gaps: `SendMessageConfiguration.historyLength`
//! wasn't applied to the task returned by `SendMessage` (only to
//! `GetTask`/`ListTasks`), and `AgentCard.capabilities.extensions[].required`
//! was declared but never checked against the caller's `A2A-Extensions`
//! header.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{
    AgentCard, AgentExtension, AgentInterface, Message, SendMessageConfiguration, TaskState,
};

/// Goes `Working` (no message) -> `Completed` (with a message), so a
/// completed task's `history` has two entries: the seed user message and
/// the completion message.
struct TwoHistoryEntriesAgent;

#[async_trait]
impl AgentExecutor for TwoHistoryEntriesAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_history_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "History Length Test Agent",
        "An A2A agent used for rusty_a2a's historyLength tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );

    let server = AgentServer::new(card, Arc::new(TwoHistoryEntriesAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn send_message_without_history_length_returns_full_history() {
    let base_url = spawn_history_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    assert_eq!(
        task.history.len(),
        2,
        "expected the seed message + completion message"
    );
}

#[tokio::test]
async fn send_message_history_length_truncates_the_returned_task() {
    let base_url = spawn_history_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        history_length: Some(1),
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("hi"), Some(config))
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    assert_eq!(
        task.history.len(),
        1,
        "historyLength: 1 should truncate to the most recent history entry"
    );
    assert_eq!(task.history[0].text(), "done");
}

#[tokio::test]
async fn send_message_history_length_zero_clears_history() {
    let base_url = spawn_history_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        history_length: Some(0),
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("hi"), Some(config))
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    assert!(task.history.is_empty());
}

// --- Required extension enforcement (spec Section 3.2.6 / 5.6) ---

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(
            TaskState::Completed,
            Some(Message::agent_text(format!("you said: {}", ctx.message.text()))),
        );
        Ok(())
    }
}

const REQUIRED_EXTENSION_URI: &str = "urn:rusty-a2a:test-extension";

async fn spawn_extension_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "Extension Required Agent",
        "An A2A agent requiring a specific extension, for rusty_a2a's extension enforcement tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.capabilities.extensions.push(AgentExtension {
        uri: REQUIRED_EXTENSION_URI.to_string(),
        description: "A required test extension.".to_string(),
        required: true,
        params: None,
    });

    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn missing_required_extension_is_rejected() {
    let base_url = spawn_extension_test_server().await;
    let client = A2aClient::new(format!("{base_url}/"));

    let err = client
        .send_message(Message::user_text("hi"), None)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::ExtensionSupportRequired(uri)) => {
            assert_eq!(uri, REQUIRED_EXTENSION_URI);
        }
        other => panic!("expected ExtensionSupportRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn declared_required_extension_is_accepted() {
    let base_url = spawn_extension_test_server().await;
    let client =
        A2aClient::new(format!("{base_url}/")).with_extensions(vec![REQUIRED_EXTENSION_URI.to_string()]);

    let result = client
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message should succeed once the required extension is declared");
    let task = result.as_task().expect("expected a task");
    assert_eq!(task.status.state, TaskState::Completed);
}

#[tokio::test]
async fn rest_binding_enforces_the_same_required_extension() {
    let base_url = spawn_extension_test_server().await;
    let http = reqwest::Client::new();

    let without_extension = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send");
    assert_eq!(without_extension.status(), 400);
    let body: serde_json::Value = without_extension.json().await.expect("response body");
    assert_eq!(
        body["error"]["details"][0]["reason"],
        "EXTENSION_SUPPORT_REQUIRED"
    );

    let with_extension = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .header("A2A-Extensions", REQUIRED_EXTENSION_URI)
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send");
    assert_eq!(with_extension.status(), 200);
}
