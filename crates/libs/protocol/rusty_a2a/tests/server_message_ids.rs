//! Covers spec Section 4.1.4: "For server messages, contextId MUST be
//! provided, and taskId only if a task was created" - both a task-less
//! bare-message reply (`EventSink::message`) and a status message attached
//! to a task update (`EventSink::status_with_message`) are stamped with
//! the invocation's ids when the executor doesn't already set them.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::A2aClient;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskState};

struct TestAgent;

#[async_trait]
impl AgentExecutor for TestAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        if ctx.message.text().contains("clarify") {
            // A task-less bare-message reply: no task is ever created, so
            // only `contextId` should be stamped, never `taskId`.
            events.message(Message::agent_text("what did you mean?"));
            return Ok(());
        }
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
        "Server Message Ids Test Agent",
        "An A2A agent used for rusty_a2a's server-message contextId/taskId stamping tests.",
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

#[tokio::test]
async fn a_bare_message_reply_carries_context_id_but_never_task_id() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("please clarify"), None)
        .await
        .expect("send_message");
    let message = result.as_message().expect("expected a bare message reply");
    assert!(
        message.context_id.is_some(),
        "spec Section 4.1.4: server messages MUST carry a contextId"
    );
    assert!(
        message.task_id.is_none(),
        "a task-less reply must never claim a taskId - no task was created"
    );
}

#[tokio::test]
async fn a_task_status_message_carries_both_context_id_and_task_id() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hello"), None)
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    let status_message = task
        .status
        .message
        .as_ref()
        .expect("expected a status message on the completed task");

    assert_eq!(status_message.context_id.as_deref(), task.context_id.as_deref());
    assert_eq!(status_message.task_id.as_deref(), Some(task.id.as_str()));

    // The same status message, once recorded into history, keeps its ids.
    let recorded = task
        .history
        .iter()
        .find(|m| m.message_id == status_message.message_id)
        .expect("expected the status message in task history");
    assert_eq!(recorded.context_id.as_deref(), task.context_id.as_deref());
    assert_eq!(recorded.task_id.as_deref(), Some(task.id.as_str()));
}
