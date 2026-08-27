//! Covers spec Section 7.6.1's out-of-band `AUTH_REQUIRED` continuation
//! pattern: "the agent SHOULD maintain any active response streams with
//! the client after setting the TaskState to `TASK_STATE_AUTH_REQUIRED`...
//! [and] MAY immediately continue Task processing after receiving the
//! credential, without a requirement that clients send a follow-up
//! message." Unlike `INPUT_REQUIRED` (whose only defined continuation is
//! a fresh client message, already covered elsewhere), `AUTH_REQUIRED`
//! credentials arrive out-of-band - the executor itself keeps running and
//! later events must still reach whoever's still listening, with no new
//! `SendMessage`/`SendStreamingMessage`/`SubscribeToTask` call involved.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use rusty_a2a::client::A2aClient;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{
    AgentCard, AgentInterface, Message, SendMessageConfiguration, StreamResponse, TaskState,
};
use tokio::sync::Notify;

/// Goes `Working` -> `AuthRequired` -> waits for `advance` (simulating a
/// credential arriving out-of-band, e.g. via an OAuth redirect the client
/// never sends a message about) -> `Completed`, all within one
/// `execute()` call.
struct OutOfBandAuthAgent {
    advance: Arc<Notify>,
}

#[async_trait]
impl AgentExecutor for OutOfBandAuthAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(
            TaskState::AuthRequired,
            Some(Message::agent_text("please authenticate")),
        );
        self.advance.notified().await;
        events.status_with_message(TaskState::Completed, Some(Message::agent_text("done")));
        Ok(())
    }
}

async fn spawn_test_server() -> (String, Arc<Notify>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let advance = Arc::new(Notify::new());

    let card = AgentCard::new(
        "Out-of-Band Auth Continuation Test Agent",
        "An A2A agent used for rusty_a2a's AUTH_REQUIRED continuation tests.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    )
    .with_streaming(true);

    let server = AgentServer::new(
        card,
        Arc::new(OutOfBandAuthAgent {
            advance: advance.clone(),
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (base_url, advance)
}

fn status_state(evt: &StreamResponse) -> Option<TaskState> {
    match evt {
        StreamResponse::StatusUpdate { status_update } => Some(status_update.status.state),
        _ => None,
    }
}

#[tokio::test]
async fn send_streaming_message_stays_open_across_auth_required() {
    let (base_url, advance) = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let mut stream = client
        .send_streaming_message(Message::user_text("hi"), None)
        .await
        .expect("send_streaming_message");

    // Leading Task snapshot (spec Section 3.1.2), then Working.
    let lead = stream.next().await.expect("lead event").expect("stream event");
    assert!(matches!(lead, StreamResponse::Task { .. }));
    let working = stream.next().await.expect("working event").expect("stream event");
    assert_eq!(status_state(&working), Some(TaskState::Working));

    let auth_required = stream
        .next()
        .await
        .expect("auth required event")
        .expect("stream event");
    assert_eq!(status_state(&auth_required), Some(TaskState::AuthRequired));

    // Simulate the credential arriving out-of-band: nothing is sent to
    // the task, the executor just resumes on its own. The SAME stream
    // must deliver the eventual completion, not close at AuthRequired.
    advance.notify_one();
    let completed = stream
        .next()
        .await
        .expect("completed event")
        .expect("stream event");
    assert_eq!(status_state(&completed), Some(TaskState::Completed));
    assert!(
        stream.next().await.is_none(),
        "stream should close after the terminal event"
    );
}

#[tokio::test]
async fn subscribe_to_task_stays_open_across_auth_required() {
    let (base_url, advance) = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let config = SendMessageConfiguration {
        return_immediately: true,
        ..Default::default()
    };
    let result = client
        .send_message(Message::user_text("hi"), Some(config))
        .await
        .expect("send_message");
    let task_id = result.as_task().expect("expected a task").id.clone();

    let mut stream = client
        .subscribe_to_task(&task_id)
        .await
        .expect("subscribe_to_task");

    // Drain events until AuthRequired (there may be a lead Task and/or
    // Working first, depending on exactly when we attached).
    let mut saw_auth_required = false;
    while let Some(evt) = stream.next().await {
        if status_state(&evt.expect("stream event")) == Some(TaskState::AuthRequired) {
            saw_auth_required = true;
            break;
        }
    }
    assert!(saw_auth_required, "expected an AuthRequired event");

    advance.notify_one();
    let mut saw_completed = false;
    while let Some(evt) = stream.next().await {
        if status_state(&evt.expect("stream event")) == Some(TaskState::Completed) {
            saw_completed = true;
            break;
        }
    }
    assert!(
        saw_completed,
        "expected the subscribe stream to deliver the eventual completion without resubscribing"
    );
}

#[tokio::test]
async fn blocking_send_message_still_returns_immediately_at_auth_required() {
    let (base_url, _advance) = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    // Blocking SendMessage has no concept of "wait for an out-of-band
    // credential" - it must still return as soon as AuthRequired is
    // reached, exactly as it does for any other interrupted state.
    let result = client
        .send_message(Message::user_text("hi"), None)
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    assert_eq!(task.status.state, TaskState::AuthRequired);
}
