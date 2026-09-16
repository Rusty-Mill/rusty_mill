//! Covers `ContentTypeNotSupportedError` (spec Section 3.3.2): a message
//! carrying a `Part.mediaType` this agent never declared support for via
//! `AgentCard.defaultInputModes` must be rejected before the executor
//! ever runs, not silently accepted.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{A2aClient, ClientError};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, Part, Role, TaskState};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

/// `AgentCard::new` defaults `defaultInputModes` to `["text/plain"]`.
async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Content Type Validation Test Agent",
        "An A2A agent used for rusty_a2a's ContentTypeNotSupportedError tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn a_part_with_an_undeclared_media_type_is_rejected() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let message = Message::new(
        Role::User,
        vec![Part::raw(b"\x89PNG...".to_vec()).with_media_type("image/png")],
    );
    let err = client.send_message(message, None).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::ContentTypeNotSupported(media_type)) => {
            assert_eq!(media_type, "image/png");
        }
        other => panic!("expected ContentTypeNotSupported, got {other:?}"),
    }
}

#[tokio::test]
async fn a_part_with_a_declared_media_type_is_accepted() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let message = Message::new(Role::User, vec![Part::text("hi").with_media_type("text/plain")]);
    let result = client.send_message(message, None).await.expect("send_message");
    assert_eq!(result.as_task().unwrap().status.state, TaskState::Completed);
}

#[tokio::test]
async fn a_part_with_no_media_type_makes_no_claim_and_is_always_accepted() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    // `Part::text` (used by `Message::user_text`) never sets `mediaType`.
    let result = client
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    assert_eq!(result.as_task().unwrap().status.state, TaskState::Completed);
}

#[tokio::test]
async fn an_agent_with_no_declared_input_modes_accepts_anything() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut card = AgentCard::new(
        "No Declared Input Modes Test Agent",
        "An A2A agent used for rusty_a2a's ContentTypeNotSupportedError tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    card.default_input_modes.clear();
    let server = AgentServer::new(card, Arc::new(EchoAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");
    let message = Message::new(
        Role::User,
        vec![Part::raw(b"\x89PNG...".to_vec()).with_media_type("image/png")],
    );
    let result = client.send_message(message, None).await.expect("send_message");
    assert_eq!(result.as_task().unwrap().status.state, TaskState::Completed);
}

#[tokio::test]
async fn rest_binding_rejects_the_same_way() {
    let base_url = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {
                "messageId": "m1",
                "role": "ROLE_USER",
                "parts": [{"raw": "iVBORw0KGgo=", "mediaType": "image/png"}]
            }
        }))
        .send()
        .await
        .expect("POST /message:send");
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(
        body["error"]["details"][0]["reason"],
        "CONTENT_TYPE_NOT_SUPPORTED"
    );
}
