//! Covers the gRPC binding's side of tenant isolation: `tenant` is a
//! plain (non-`Option`) `String` on the generated types, with `""` as the
//! proto3 zero-value standing in for "unset" (see
//! `src/server/grpc/convert.rs`'s `non_empty` helper).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::error::Result;
use rusty_a2a::server::grpc::pb;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskState};
use tonic::transport::Channel;

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
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    drop(listener);

    let card = AgentCard::new(
        "gRPC Tenant Isolation Test Agent",
        "An A2A agent used for rusty_a2a's gRPC tenant isolation test.",
        "0.0.0",
        AgentInterface::json_rpc(format!("http://127.0.0.1:{port}")),
    );

    let services = AgentServer::new(card, Arc::new(EchoAgent)).build();
    tokio::spawn(async move {
        services.serve_grpc(([127, 0, 0, 1], port)).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    format!("http://127.0.0.1:{port}")
}

async fn connect(url: String) -> pb::a2a_service_client::A2aServiceClient<Channel> {
    pb::a2a_service_client::A2aServiceClient::connect(url)
        .await
        .expect("connect")
}

fn user_message(text: &str) -> pb::Message {
    pb::Message {
        message_id: "m1".to_string(),
        role: pb::Role::User as i32,
        parts: vec![pb::Part {
            content: Some(pb::part::Content::Text(text.to_string())),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn grpc_task_created_under_one_tenant_is_invisible_to_another() {
    let url = spawn_test_server().await;
    let mut client = connect(url).await;

    let started = client
        .send_message(pb::SendMessageRequest {
            message: Some(user_message("hi")),
            tenant: "tenant-a".to_string(),
            ..Default::default()
        })
        .await
        .expect("send_message")
        .into_inner();
    let task_id = match started.payload {
        Some(pb::send_message_response::Payload::Task(task)) => task.id,
        other => panic!("expected a task, got {other:?}"),
    };

    // Same tenant: found.
    let same_tenant = client
        .get_task(pb::GetTaskRequest {
            id: task_id.clone(),
            tenant: "tenant-a".to_string(),
            ..Default::default()
        })
        .await;
    assert!(same_tenant.is_ok());

    // Different tenant: NotFound, not a leak.
    let other_tenant = client
        .get_task(pb::GetTaskRequest {
            id: task_id.clone(),
            tenant: "tenant-b".to_string(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(other_tenant.code(), tonic::Code::NotFound);

    // No tenant at all: also NotFound - the task only exists under
    // "tenant-a", not in the default/no-tenant namespace.
    let no_tenant = client
        .get_task(pb::GetTaskRequest {
            id: task_id,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(no_tenant.code(), tonic::Code::NotFound);
}
