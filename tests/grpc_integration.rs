//! End-to-end test for the gRPC binding: spins up a real
//! `AgentServices::serve_grpc` on a local TCP port and drives it with a
//! real `tonic` client generated from the same `spec/a2a.proto`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::error::Result;
use rusty_a2a::server::grpc::pb;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Artifact, Message, Part, TaskState};
use tonic::transport::Channel;

struct TestAgent;

#[async_trait]
impl AgentExecutor for TestAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        let text = ctx.message.text();

        if text.contains("clarify") {
            events.message(Message::agent_text("what did you mean by that?"));
            return Ok(());
        }

        events.status(TaskState::Working);
        events.artifact(Artifact::new("result", vec![Part::text("42")]));
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> (String, u16) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    drop(listener);

    let card = AgentCard::new(
        "gRPC Test Agent",
        "An A2A agent used for rusty_a2a's gRPC integration tests.",
        "0.0.0",
        AgentInterface::json_rpc(format!("http://127.0.0.1:{port}")),
    )
    .with_streaming(true)
    .with_push_notifications(true);

    let services = AgentServer::new(card, Arc::new(TestAgent)).build();
    tokio::spawn(async move {
        services.serve_grpc(([127, 0, 0, 1], port)).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    (format!("http://127.0.0.1:{port}"), port)
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
async fn send_message_and_get_task_round_trip() {
    let (url, _) = spawn_test_server().await;
    let mut client = connect(url).await;

    let response = client
        .send_message(pb::SendMessageRequest {
            message: Some(user_message("please compute")),
            ..Default::default()
        })
        .await
        .expect("send_message")
        .into_inner();

    let task = match response.payload {
        Some(pb::send_message_response::Payload::Task(task)) => task,
        other => panic!("expected a task, got {other:?}"),
    };
    assert_eq!(
        task.status.as_ref().unwrap().state,
        pb::TaskState::Completed as i32
    );
    assert_eq!(task.artifacts.len(), 1);

    let fetched = client
        .get_task(pb::GetTaskRequest {
            id: task.id.clone(),
            ..Default::default()
        })
        .await
        .expect("get_task")
        .into_inner();
    assert_eq!(fetched.id, task.id);
    assert_eq!(fetched.status.unwrap().state, pb::TaskState::Completed as i32);
}

#[tokio::test]
async fn message_only_reply_has_no_task() {
    let (url, _) = spawn_test_server().await;
    let mut client = connect(url).await;

    let response = client
        .send_message(pb::SendMessageRequest {
            message: Some(user_message("please clarify this")),
            ..Default::default()
        })
        .await
        .expect("send_message")
        .into_inner();

    match response.payload {
        Some(pb::send_message_response::Payload::Message(m)) => {
            assert_eq!(
                m.parts[0].content,
                Some(pb::part::Content::Text("what did you mean by that?".to_string()))
            );
        }
        other => panic!("expected a bare message, got {other:?}"),
    }
}

#[tokio::test]
async fn streaming_message_yields_ordered_events() {
    use futures_util::StreamExt;

    let (url, _) = spawn_test_server().await;
    let mut client = connect(url).await;

    let mut stream = client
        .send_streaming_message(pb::SendMessageRequest {
            message: Some(user_message("please compute")),
            ..Default::default()
        })
        .await
        .expect("send_streaming_message")
        .into_inner();

    let mut saw_working = false;
    let mut saw_artifact = false;
    let mut saw_completed = false;
    while let Some(event) = stream.next().await {
        match event.expect("stream event").payload {
            Some(pb::stream_response::Payload::StatusUpdate(u)) => {
                match pb::TaskState::try_from(u.status.unwrap().state).unwrap() {
                    pb::TaskState::Working => saw_working = true,
                    pb::TaskState::Completed => saw_completed = true,
                    other => panic!("unexpected state {other:?}"),
                }
            }
            Some(pb::stream_response::Payload::ArtifactUpdate(_)) => saw_artifact = true,
            other => panic!("unexpected stream item: {other:?}"),
        }
    }
    assert!(saw_working && saw_artifact && saw_completed);
}

#[tokio::test]
async fn task_not_found_maps_to_not_found_status() {
    let (url, _) = spawn_test_server().await;
    let mut client = connect(url).await;

    let err = client
        .get_task(pb::GetTaskRequest {
            id: "does-not-exist".to_string(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn push_notification_config_crud() {
    let (url, _) = spawn_test_server().await;
    let mut client = connect(url).await;

    let task = match client
        .send_message(pb::SendMessageRequest {
            message: Some(user_message("please compute")),
            ..Default::default()
        })
        .await
        .expect("send_message")
        .into_inner()
        .payload
    {
        Some(pb::send_message_response::Payload::Task(task)) => task,
        other => panic!("expected a task, got {other:?}"),
    };

    let created = client
        .create_task_push_notification_config(pb::TaskPushNotificationConfig {
            task_id: task.id.clone(),
            url: "https://example.com/webhook".to_string(),
            ..Default::default()
        })
        .await
        .expect("create push config")
        .into_inner();
    assert!(!created.id.is_empty());

    let listed = client
        .list_task_push_notification_configs(pb::ListTaskPushNotificationConfigsRequest {
            task_id: task.id.clone(),
            ..Default::default()
        })
        .await
        .expect("list push configs")
        .into_inner();
    assert_eq!(listed.configs.len(), 1);

    client
        .delete_task_push_notification_config(pb::DeleteTaskPushNotificationConfigRequest {
            task_id: task.id.clone(),
            id: created.id.clone(),
            ..Default::default()
        })
        .await
        .expect("delete push config");

    let err = client
        .get_task_push_notification_config(pb::GetTaskPushNotificationConfigRequest {
            task_id: task.id,
            id: created.id,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}
