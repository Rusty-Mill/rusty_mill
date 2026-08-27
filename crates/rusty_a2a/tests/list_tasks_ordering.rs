//! Covers spec Section 3.1.4's `ListTasks` ordering requirement:
//! "Implementations MUST return tasks sorted by their status timestamp
//! time in descending order (most recently updated tasks first)."

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::A2aClient;
use rusty_a2a::error::Result;
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "List Tasks Ordering Test Agent",
        "An A2A agent used for rusty_a2a's ListTasks ordering tests.",
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
async fn list_tasks_orders_by_status_timestamp_descending() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let mut created = Vec::new();
    for _ in 0..3 {
        let result = client
            .send_message(Message::user_text("hello"), None)
            .await
            .expect("send_message");
        created.push(result.as_task().expect("expected a task").id.clone());
        // Ensure each task's status timestamp is strictly later than the
        // previous one, so descending order is unambiguous.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let listed = client.list_tasks(Default::default()).await.expect("list_tasks");
    let observed: Vec<&str> = listed
        .tasks
        .iter()
        .filter(|t| created.contains(&t.id))
        .map(|t| t.id.as_str())
        .collect();

    let expected: Vec<&str> = created.iter().rev().map(String::as_str).collect();
    assert_eq!(
        observed, expected,
        "expected most-recently-updated task first, got {observed:?}"
    );
}
