//! Recording `cancelling` must hand off to the executor, not race it.
//!
//! Two tasks on the executing replica can write a run's snapshot: the control
//! listener, which records `cancelling` when a cancel arrives, and the executor,
//! which writes the terminal `cancelled` once it notices. The sole-writer
//! invariant says only one thing writes a run — within a replica that means
//! these two must be ordered, not concurrent.
//!
//! Signalling the cancellation token before the `cancelling` write has landed
//! makes them concurrent: the executor wakes immediately and its terminal write
//! races the listener's. When the listener's lands second the store is left on
//! `cancelling` — non-terminal — after the executor has finished and released
//! its lease, so the run is later reaped as abandoned and reported `failed`.
//!
//! In-process the listener's write almost always wins by default. Where a write
//! is a network round-trip it frequently does not. So this is made observable
//! rather than raced: the store below delays exactly the `cancelling` write, so
//! the losing order happens every time.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use rusty_acp::client::{AcpClient, WaitOptions};
use rusty_acp::server::store::{
    InMemoryStore, Notification, NotificationStream, RecoveryRecord, SessionRecord, Store,
    StoreResult,
};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, Event, Message, Run, RunId, RunStatus, Session, SessionId,
};

/// How long the `cancelling` write is held up — long enough that an executor
/// woken early would certainly finish first.
const CANCELLING_WRITE_DELAY: Duration = Duration::from_millis(400);

/// An [`InMemoryStore`] that is slow to record `cancelling`, and only that.
///
/// Every other write, including the terminal one, goes through untouched — so
/// the only thing this changes is which of the two writers lands last.
#[derive(Debug)]
struct SlowCancellingWriteStore(InMemoryStore);

#[async_trait::async_trait]
impl Store for SlowCancellingWriteStore {
    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        if run.status == RunStatus::Cancelling {
            tokio::time::sleep(CANCELLING_WRITE_DELAY).await;
        }
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
    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        self.0.append_session_messages(session_id, base_url, messages).await
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

/// Two replicas over one slow-cancelling-write store.
async fn replicas() -> (AcpClient, AcpClient) {
    let store: Arc<dyn Store> = Arc::new(SlowCancellingWriteStore(InMemoryStore::new(1024)));
    (replica(Arc::clone(&store)).await, replica(store).await)
}

async fn replica(store: Arc<dyn Store>) -> AcpClient {
    let forever = agent_fn(
        AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes on its own"),
        |ctx: RunContext| async move {
            ctx.cancelled().await;
            Ok(())
        },
    );

    let greeter = agent_fn(
        AgentManifest::new(AgentName::new("greeter").unwrap(), "Waits for an answer"),
        |ctx: RunContext| async move {
            ctx.await_json(serde_json::json!({ "question": "name?" })).await?;
            ctx.reply_text("hello").await?;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(forever)
        .agent(greeter)
        .store(store)
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    AcpClient::new(format!("http://{addr}")).unwrap()
}

/// A cancelled run ends `cancelled`, even when recording `cancelling` is slow.
///
/// The wait is the point. `cancel_and_wait` returns as soon as it *sees* a
/// terminal status, which it does before a late `cancelling` write lands — so
/// asserting on its return value would miss the bug entirely. What matters is
/// what the store says once every write has settled.
#[tokio::test]
async fn a_slow_cancelling_write_cannot_outlive_the_terminal_one() {
    let (a, b) = replicas().await;

    let started = a.run_async("forever", [Message::user("hang")]).await.unwrap();

    b.cancel_and_wait(started.run_id, WaitOptions::default().with_timeout(Duration::from_secs(10)))
        .await
        .unwrap();

    settle().await;

    let seen = a.get_run(started.run_id).await.unwrap();
    assert_eq!(
        seen.status,
        RunStatus::Cancelled,
        "the terminal write must be the last word on the run, not whichever write happened to land last"
    );
}

/// Long enough for a delayed `cancelling` write to have landed.
async fn settle() {
    tokio::time::sleep(CANCELLING_WRITE_DELAY * 2).await;
}

/// And the run stays cancelled — it is not later reaped as abandoned.
///
/// This is the symptom the ordering bug actually produces: a run left on the
/// non-terminal `cancelling` after its executor has finished and released its
/// lease looks, to the next replica that reads it, exactly like one whose
/// replica died.
#[tokio::test]
async fn a_cancelled_run_is_not_then_reaped_as_abandoned() {
    let (a, b) = replicas().await;

    let started = a.run_async("forever", [Message::user("hang")]).await.unwrap();
    b.cancel_and_wait(started.run_id, WaitOptions::default().with_timeout(Duration::from_secs(10)))
        .await
        .unwrap();

    settle().await;

    // Read it again through the other replica, which is what triggers reaping.
    let seen = b.get_run(started.run_id).await.unwrap();
    assert_eq!(seen.status, RunStatus::Cancelled);
    assert!(
        seen.error.is_none(),
        "a run cancelled on request must not carry an abandonment error: {:?}",
        seen.error
    );
}

/// The same for a run cancelled while it is awaiting client input.
#[tokio::test]
async fn cancelling_an_awaiting_run_still_ends_cancelled() {
    let (a, b) = replicas().await;

    let paused = a.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);

    b.cancel_and_wait(paused.run_id, WaitOptions::default().with_timeout(Duration::from_secs(10)))
        .await
        .unwrap();

    settle().await;

    let seen = b.get_run(paused.run_id).await.unwrap();
    assert_eq!(seen.status, RunStatus::Cancelled);
    assert!(seen.error.is_none(), "{:?}", seen.error);
}
