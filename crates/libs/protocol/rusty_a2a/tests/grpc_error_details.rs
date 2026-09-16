//! Covers spec Section 10.6's `google.rpc.ErrorInfo` requirement for gRPC
//! errors: "implementations MUST include a `google.rpc.ErrorInfo` message
//! in the `status.details` array" for the nine A2A-specific errors. Before
//! this, the gRPC binding sent a bare `tonic::Status` (code + message
//! only), so `client::GrpcClient` could only guess at the specific
//! `A2aError` from the gRPC `Code` alone - and several distinct A2aErrors
//! share the same code (spec Section 5.4's "gRPC Status" column), so a
//! guess from the code alone is ambiguous. This file proves the
//! disambiguation actually works, using two errors that both map to
//! `FailedPrecondition`: `TaskNotCancelable` and `UnsupportedOperation`.

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

async fn spawn_test_server() -> String {
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let http_base_url = format!("http://{http_addr}");

    let grpc_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    drop(grpc_listener);
    let grpc_url = format!("http://{grpc_addr}");

    let card = AgentCard::new(
        "gRPC Error Details Test Agent",
        "An A2A agent used for rusty_a2a's gRPC ErrorInfo tests.",
        "0.0.0",
        AgentInterface::json_rpc(http_base_url.clone()),
    )
    .with_interface(AgentInterface::grpc(grpc_url));

    let services = AgentServer::new(card, Arc::new(EchoAgent)).build();
    let http_services = services.clone();
    tokio::spawn(async move {
        axum::serve(http_listener, http_services.router()).await.unwrap();
    });
    tokio::spawn(async move {
        services.serve_grpc(grpc_addr).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    http_base_url
}

#[tokio::test]
async fn cancel_task_on_a_terminal_task_reconstructs_task_not_cancelable() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();
    assert_eq!(result.as_task().unwrap().status.state, TaskState::Completed);

    let err = client.cancel_task(&task_id).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::TaskNotCancelable(id)) => assert_eq!(id, task_id),
        other => panic!("expected TaskNotCancelable, got {other:?}"),
    }
}

#[tokio::test]
async fn send_message_to_a_terminal_task_reconstructs_unsupported_operation() {
    let base_url = spawn_test_server().await;
    let (client, _) = GrpcClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    // Same gRPC code (`FailedPrecondition`) as `TaskNotCancelable` above,
    // but a different reason - proving the two are actually
    // distinguished, not just both landing on a shared fallback guess.
    let continuation = Message::user_text("still there?").with_task_id(&task_id);
    let err = client.send_message(continuation, None).await.unwrap_err();
    match err {
        ClientError::Protocol(A2aError::UnsupportedOperation(_)) => {}
        other => panic!("expected UnsupportedOperation, got {other:?}"),
    }
}
