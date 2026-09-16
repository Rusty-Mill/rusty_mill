//! Covers spec Sections 3.2.6/3.6.2's `A2A-Version` service parameter
//! being enforced identically across all three bindings (spec Section
//! 5.1's "same error handling" requirement for every binding an agent
//! exposes) - not just JSON-RPC (already covered by
//! `tests/integration.rs`'s `version_header_mismatch_is_rejected`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::{ClientError, GrpcClient};
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskState};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

/// Spins up JSON-RPC+REST (one port) and gRPC (another port), returning
/// the HTTP base URL.
async fn spawn_test_server() -> (String, String) {
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let http_base_url = format!("http://{http_addr}");

    let grpc_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    drop(grpc_listener);
    let grpc_url = format!("http://{grpc_addr}");

    let card = AgentCard::new(
        "Version Negotiation Test Agent",
        "An A2A agent used for rusty_a2a's A2A-Version enforcement tests.",
        "0.0.0",
        AgentInterface::json_rpc(http_base_url.clone()),
    )
    .with_interface(AgentInterface::http_json(http_base_url.clone()))
    .with_interface(AgentInterface::grpc(grpc_url.clone()));

    let services = AgentServer::new(card, Arc::new(EchoAgent)).build();
    let http_services = services.clone();
    tokio::spawn(async move {
        axum::serve(http_listener, http_services.router()).await.unwrap();
    });
    tokio::spawn(async move {
        services.serve_grpc(grpc_addr).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (http_base_url, grpc_url)
}

#[tokio::test]
async fn rest_binding_rejects_a_mismatched_version_header() {
    let (base_url, _) = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{base_url}/message:send"))
        .header("A2A-Version", "0.3")
        .json(&serde_json::json!({
            "message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "hi"}]}
        }))
        .send()
        .await
        .expect("POST /message:send");
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.expect("response body");
    assert_eq!(body["error"]["status"], "FAILED_PRECONDITION");
    assert_eq!(body["error"]["details"][0]["reason"], "VERSION_NOT_SUPPORTED");
}

#[tokio::test]
async fn rest_extended_agent_card_also_enforces_version() {
    let (base_url, _) = spawn_test_server().await;
    let http = reqwest::Client::new();

    let resp = http
        .get(format!("{base_url}/extendedAgentCard"))
        .header("A2A-Version", "not-a-real-version")
        .send()
        .await
        .expect("GET /extendedAgentCard");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn grpc_binding_rejects_a_mismatched_version_metadata_entry() {
    let (base_url, _grpc_url) = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");
    let client = client.with_protocol_version("0.3");

    // The server attaches a `google.rpc.ErrorInfo` detail with the same
    // `VERSION_NOT_SUPPORTED` reason JSON-RPC/REST use, so the client
    // reconstructs the precise `A2aError` variant, not just a guess from
    // the bare `FailedPrecondition` code (which several other A2aErrors
    // also map to).
    let err = client
        .send_message(Message::user_text("hi"), None)
        .await
        .unwrap_err();
    match err {
        ClientError::Protocol(A2aError::VersionNotSupported(v)) => assert_eq!(v, "0.3"),
        other => panic!("expected VersionNotSupported, got {other:?}"),
    }
}
