//! What a client is allowed to observe once it has been told a run is done.
//!
//! A `sync` caller is released by the terminal event. Everything that caller
//! could reasonably read next — most sharply, the session history it is about
//! to continue the conversation from — has to be in place *before* that event
//! goes out.
//!
//! Timing alone cannot test this. The gap is normally microseconds wide, so a
//! test that races it passes by luck on a fast store and fails intermittently
//! on a slow one, which is exactly how the bug reached `main` in the first
//! place. Instead the store here makes the ordering observable: appending to a
//! session takes a beat, so a run that is marked terminal before its history is
//! written is caught every time rather than occasionally.

#![cfg(feature = "server")]

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{
    InMemoryStore, Notification, NotificationStream, RecoveryRecord, SessionRecord, Store,
    StoreResult,
};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, Event, Message, Run, RunCreateRequest, RunId, RunStatus, Session,
    SessionId,
};

/// How long an append is made to take. Long enough that losing the race is
/// certain rather than likely, short enough not to slow the suite down.
const APPEND_DELAY: Duration = Duration::from_millis(300);

/// An [`InMemoryStore`] whose session appends are slow.
///
/// Every other operation delegates untouched, so the only thing this changes is
/// how much room there is between writing history and writing the run's
/// terminal state.
#[derive(Debug)]
struct SlowAppendStore(InMemoryStore);

#[async_trait::async_trait]
impl Store for SlowAppendStore {
    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        tokio::time::sleep(APPEND_DELAY).await;
        self.0.append_session_messages(session_id, base_url, messages).await
    }

    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        self.0.put_run(run).await
    }
    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        self.0.get_run(run_id).await
    }
    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
        self.0.append_event(run_id, event).await
    }
    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        self.0.events(run_id).await
    }
    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        self.0.events_from(run_id, from).await
    }
    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        self.0.publish(run_id, notification).await
    }
    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        self.0.subscribe(run_id).await
    }
    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        self.0.get_session(session_id).await
    }
    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        self.0.ensure_session(session).await
    }
    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        self.0.get_session_state(session_id).await
    }
    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        self.0.put_session_state(session_id, base_url, state).await
    }
    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
        self.0.renew_lease(run_id, owner, ttl).await
    }
    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        self.0.lease_owner(run_id).await
    }
    async fn try_claim_lease(
        &self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> StoreResult<bool> {
        self.0.try_claim_lease(run_id, owner, ttl).await
    }
    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        self.0.put_recovery_record(run_id, record).await
    }
    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        self.0.recovery_record(run_id).await
    }
    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        self.0.release_lease(run_id).await
    }
}

/// A server with two agents: one that closes its message, one that does not.
async fn start_server() -> AcpClient {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
        |ctx: RunContext| async move {
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    // Returns without calling `finish`, so its output only exists once the
    // trailing message has been flushed. If the flush and the history write
    // fall on opposite sides of the terminal event, this agent's output goes
    // missing rather than merely arriving late.
    let trailing = agent_fn(
        AgentManifest::new(AgentName::new("trailing").unwrap(), "Leaves its message open"),
        |ctx: RunContext| async move {
            let mut writer = ctx.begin_message().await?;
            writer.push_text("half").await?;
            writer.push_text(" said").await?;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(echo)
        .agent(trailing)
        .store(Arc::new(SlowAppendStore(InMemoryStore::new(1024))))
        .build()
        .unwrap()
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    AcpClient::new(format!("http://{addr}")).unwrap()
}

/// A completed `sync` run's output is in the session by the time the client
/// hears about it.
#[tokio::test]
async fn a_sync_run_is_not_completed_before_its_history_is() {
    let client = start_server().await;
    let session_id = SessionId::new();

    let run = client
        .create_run(
            RunCreateRequest::new(AgentName::new("echo").unwrap(), [Message::user("hello")])
                .with_session_id(session_id),
        )
        .await
        .unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    // No polling and no grace period: the run says it is done, so the history
    // behind it must already be complete.
    let session = client.get_session(session_id).await.unwrap();
    assert_eq!(
        session.history.len(),
        2,
        "a completed run's input and output must both be in the session"
    );

    let messages = client.fetch_session_history(&session).await.unwrap();
    let texts: Vec<_> = messages.iter().map(|message| message.text()).collect();
    assert_eq!(texts, ["hello", "hello"]);
}

/// The same, for an agent that returned mid-message.
///
/// Its output does not exist until the trailing message is flushed, which is
/// why the flush has to happen before the history is written rather than as
/// part of marking the run terminal.
#[tokio::test]
async fn a_trailing_message_reaches_the_session_too() {
    let client = start_server().await;
    let session_id = SessionId::new();

    let run = client
        .create_run(
            RunCreateRequest::new(AgentName::new("trailing").unwrap(), [Message::user("go")])
                .with_session_id(session_id),
        )
        .await
        .unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output_text(), "half said");

    let session = client.get_session(session_id).await.unwrap();
    assert_eq!(session.history.len(), 2, "the flushed message must be in the session");

    let messages = client.fetch_session_history(&session).await.unwrap();
    assert_eq!(messages[1].text(), "half said");
}

/// The input side of the same rule.
///
/// A streaming client is woken by `run.created`, which is published before the
/// run has finished anything — so that event, too, must not arrive ahead of the
/// history it implies. Asserted through the stream rather than through a `sync`
/// or `async` response, because only a subscriber can observe the moment the
/// run is announced.
#[tokio::test]
async fn the_input_is_in_the_session_before_the_run_is_announced() {
    let client = start_server().await;
    let session_id = SessionId::new();

    let mut stream = client
        .stream_run(
            RunCreateRequest::new(AgentName::new("echo").unwrap(), [Message::user("first")])
                .with_session_id(session_id),
        )
        .await
        .unwrap();

    // Stop at the announcement. The run has been created, so the input it was
    // created from has to be readable already.
    let announced = stream.next().await.expect("the stream must open").unwrap();
    assert!(
        matches!(announced, Event::RunCreated { .. }),
        "expected run.created first, got {announced:?}"
    );

    let session = client.get_session(session_id).await.unwrap();
    assert_eq!(
        session.history.len(),
        1,
        "the input must be in the session before the run is announced"
    );
}
