//! Covers the gRPC binding's version of `SubscribeToTask` replay: since
//! the canonical `SubscribeToTaskRequest` has no `Last-Event-ID`-style
//! resume field, a gRPC resubscribe always replays a task's *entire*
//! buffered event log (unlike JSON-RPC/REST, which can resume precisely
//! via the `Last-Event-ID` SSE header - see `tests/subscribe_replay.rs`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use rusty_a2a::error::Result;
use rusty_a2a::server::grpc::pb;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Artifact, Message, Part, TaskState};
use tokio::sync::Notify;
use tonic::transport::Channel;

struct SteppedAgent {
    advance: Arc<Notify>,
}

#[async_trait]
impl AgentExecutor for SteppedAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        self.advance.notified().await;
        events.artifact(Artifact::new("result", vec![Part::text("42")]));
        self.advance.notified().await;
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> (String, Arc<Notify>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    drop(listener);

    let advance = Arc::new(Notify::new());
    let card = AgentCard::new(
        "gRPC Subscribe Replay Test Agent",
        "An A2A agent used for rusty_a2a's gRPC SubscribeToTask replay test.",
        "0.0.0",
        AgentInterface::json_rpc(format!("http://127.0.0.1:{port}")),
    )
    .with_streaming(true);

    let services = AgentServer::new(
        card,
        Arc::new(SteppedAgent {
            advance: advance.clone(),
        }),
    )
    .build();
    tokio::spawn(async move {
        services.serve_grpc(([127, 0, 0, 1], port)).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    (format!("http://127.0.0.1:{port}"), advance)
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
async fn grpc_resubscribe_replays_the_whole_buffered_log() {
    let (url, advance) = spawn_test_server().await;
    let mut client = connect(url).await;

    let started = client
        .send_message(pb::SendMessageRequest {
            message: Some(user_message("hi")),
            configuration: Some(pb::SendMessageConfiguration {
                return_immediately: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("send_message")
        .into_inner();
    let task_id = match started.payload {
        Some(pb::send_message_response::Payload::Task(task)) => task.id,
        other => panic!("expected a task, got {other:?}"),
    };

    // First subscribe: read the `Working` event, then disconnect by
    // dropping the stream without reading further.
    let mut first_stream = client
        .subscribe_to_task(pb::SubscribeToTaskRequest {
            id: task_id.clone(),
            ..Default::default()
        })
        .await
        .expect("subscribe_to_task")
        .into_inner();
    let first = first_stream
        .next()
        .await
        .expect("first event")
        .expect("stream item");
    match first.payload {
        Some(pb::stream_response::Payload::StatusUpdate(u)) => {
            assert_eq!(u.status.unwrap().state, pb::TaskState::Working as i32);
        }
        other => panic!("expected a status update, got {other:?}"),
    }
    drop(first_stream);

    // Advance to the artifact update while disconnected.
    advance.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Resubscribing has no way to say "only what I missed" over gRPC, so
    // it replays from the very start of the buffered log: `Working`
    // again, then the artifact update.
    let mut second_stream = client
        .subscribe_to_task(pb::SubscribeToTaskRequest {
            id: task_id.clone(),
            ..Default::default()
        })
        .await
        .expect("subscribe_to_task (resubscribe)")
        .into_inner();

    let replayed_working = second_stream
        .next()
        .await
        .expect("replayed working event")
        .expect("stream item");
    match replayed_working.payload {
        Some(pb::stream_response::Payload::StatusUpdate(u)) => {
            assert_eq!(u.status.unwrap().state, pb::TaskState::Working as i32);
        }
        other => panic!("expected the replayed working status, got {other:?}"),
    }

    let replayed_artifact = second_stream
        .next()
        .await
        .expect("replayed artifact event")
        .expect("stream item");
    assert!(matches!(
        replayed_artifact.payload,
        Some(pb::stream_response::Payload::ArtifactUpdate(_))
    ));

    // Let the agent finish; the live tail must deliver completion.
    advance.notify_one();
    let completion = second_stream
        .next()
        .await
        .expect("completion event")
        .expect("stream item");
    match completion.payload {
        Some(pb::stream_response::Payload::StatusUpdate(u)) => {
            assert_eq!(u.status.unwrap().state, pb::TaskState::Completed as i32);
        }
        other => panic!("expected the completion status, got {other:?}"),
    }
    assert!(second_stream.next().await.is_none());
}
