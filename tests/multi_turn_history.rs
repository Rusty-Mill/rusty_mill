//! Covers a bug in continuing a task across turns (e.g. answering
//! `InputRequired`): `Engine::apply_event` used to only seed a task's
//! history with the inbound message when the task was *first created* -
//! a continuation turn found the task already in the store from its
//! earlier turn(s) and silently dropped the client's new message from
//! history, recording only the agent's own replies.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::client::A2aClient;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, Message, TaskState};

/// First turn (no existing task): asks a clarifying question via
/// `InputRequired`. Any later turn (task already exists): completes.
struct TwoTurnAgent;

#[async_trait]
impl AgentExecutor for TwoTurnAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        if ctx.task.is_none() {
            events.status_with_message(
                TaskState::InputRequired,
                Some(Message::agent_text("what's your name?")),
            );
        } else {
            events.status_with_message(
                TaskState::Completed,
                Some(Message::agent_text(format!("hello, {}", ctx.message.text()))),
            );
        }
        Ok(())
    }
}

async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let card = AgentCard::new(
        "Two Turn Test Agent",
        "An A2A agent used for rusty_a2a's multi-turn history test.",
        "0.0.0",
        AgentInterface::json_rpc(base_url.clone()),
    );
    let server = AgentServer::new(card, Arc::new(TwoTurnAgent));
    tokio::spawn(async move {
        axum::serve(listener, server.into_router()).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    base_url
}

#[tokio::test]
async fn continuing_a_task_retains_both_turns_messages_in_history() {
    let base_url = spawn_test_server().await;
    let (client, _) = A2aClient::discover(&base_url).await.expect("discover");

    let result = client
        .send_message(Message::user_text("hi there"), None)
        .await
        .expect("send_message");
    let task = result.as_task().expect("expected a task");
    assert_eq!(task.status.state, TaskState::InputRequired);
    let task_id = task.id.clone();

    let mut follow_up = Message::user_text("Ada");
    follow_up.task_id = Some(task_id.clone());
    let result2 = client
        .send_message(follow_up, None)
        .await
        .expect("send_message (continuation)");
    let task2 = result2.as_task().expect("expected a task");
    assert_eq!(task2.status.state, TaskState::Completed);

    let fetched = client.get_task(&task_id, None).await.expect("get_task");
    let texts: Vec<String> = fetched.history.iter().map(|m| m.text().to_string()).collect();
    assert_eq!(
        texts,
        vec!["hi there", "what's your name?", "Ada", "hello, Ada",],
        "expected both turns' messages interleaved in order, got {texts:?}"
    );
}
