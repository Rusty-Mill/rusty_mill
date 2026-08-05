//! Failing an abandoned run must never overwrite a run that finished on its own.
//!
//! Reaping reads a run, finds no live lease, claims the run, and writes
//! `failed`. Between that read and that claim the executing replica can reach a
//! terminal state — writing its own outcome and releasing its lease, which is
//! precisely what lets the claim succeed. Failing from the stale snapshot then
//! rewrites a completed or cancelled run, which the terminal-once rule forbids.
//!
//! Racing it would be no test at all: the window is microseconds on an
//! in-process store. So, as in `ordering.rs`, the store makes it observable —
//! a slow lease lookup holds the window open for a fixed 400ms while the run
//! finishes inside it.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{
    InMemoryStore, Notification, NotificationStream, RecoveryRecord, SessionRecord, Store,
    StoreResult,
};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, Event, Message, Run, RunId, RunStatus, Session, SessionId,
};

/// How long the lease lookup is held open. Longer than the run takes to finish,
/// so the run is always terminal by the time the lookup answers.
const LEASE_LOOKUP_DELAY: Duration = Duration::from_millis(400);

/// How long the agent runs. Comfortably inside the lookup delay.
const RUN_DURATION: Duration = Duration::from_millis(100);

/// An [`InMemoryStore`] whose lease lookups are slow.
///
/// Only `lease_owner` is touched. That is the read a reaper makes to decide
/// whether a run still has a writer, and so the only place the window between
/// "no live lease" and "claim it" can be widened.
#[derive(Debug)]
struct SlowLeaseLookupStore(InMemoryStore);

#[async_trait::async_trait]
impl Store for SlowLeaseLookupStore {
    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        tokio::time::sleep(LEASE_LOOKUP_DELAY).await;
        self.0.lease_owner(run_id).await
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

/// Two replicas over one slow-lease store: one runs the agent, one observes.
async fn replicas() -> (AcpClient, AcpClient) {
    let store: Arc<dyn Store> = Arc::new(SlowLeaseLookupStore(InMemoryStore::new(1024)));
    let a = replica(Arc::clone(&store)).await;
    let b = replica(store).await;
    (a, b)
}

async fn replica(store: Arc<dyn Store>) -> AcpClient {
    let brief = agent_fn(
        AgentManifest::new(AgentName::new("brief").unwrap(), "Finishes after a moment"),
        |ctx: RunContext| async move {
            tokio::time::sleep(RUN_DURATION).await;
            ctx.reply_text("done").await?;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(brief)
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

/// A run that completes while a reaper is mid-decision keeps its own outcome.
#[tokio::test]
async fn a_run_that_finishes_mid_reap_is_not_overwritten() {
    let (a, b) = replicas().await;

    // Runs on A, and will finish in RUN_DURATION.
    let started = a.run_async("brief", [Message::user("go")]).await.unwrap();

    // B asks about it immediately, while it is still in flight. That read takes
    // the lease lookup's LEASE_LOOKUP_DELAY, during which the run finishes on A
    // and releases its lease — so by the time B's lookup answers "no owner",
    // the run it was asked about is already completed.
    let seen = b.get_run(started.run_id).await.unwrap();

    assert_eq!(
        seen.status,
        RunStatus::Completed,
        "a run that finished on its own must not be rewritten as failed by a reaper \
         acting on a stale snapshot"
    );
    assert_eq!(seen.output_text(), "done", "its output must survive too");
    assert!(seen.error.is_none(), "a completed run must carry no abandonment error");
}

/// And the run's own terminal event is the one in the log — no `run.failed`
/// appended after it.
#[tokio::test]
async fn no_failure_event_is_appended_after_the_real_outcome() {
    let (a, b) = replicas().await;

    let started = a.run_async("brief", [Message::user("go")]).await.unwrap();
    b.get_run(started.run_id).await.unwrap();

    let events = b.list_run_events(started.run_id).await.unwrap();
    let terminal: Vec<&Event> = events.iter().filter(|event| event.is_terminal()).collect();

    assert_eq!(
        terminal.len(),
        1,
        "exactly one terminal event, or a client streaming the run sees it end twice: {terminal:?}"
    );
    assert!(matches!(terminal[0], Event::RunCompleted { .. }));
}

/// The reaper still does its job when the run really was abandoned.
///
/// The guard added for the case above must not turn into "never fail
/// anything": a non-terminal run whose lease has lapsed is still reaped.
#[tokio::test]
async fn an_abandoned_run_is_still_failed() {
    let store: Arc<dyn Store> = Arc::new(SlowLeaseLookupStore(InMemoryStore::new(1024)));
    let observer = replica(Arc::clone(&store)).await;

    // A run nobody is executing and nobody holds a lease on: exactly what a
    // dead replica leaves behind.
    let mut stranded = Run::new(AgentName::new("brief").unwrap(), None);
    stranded.status = RunStatus::InProgress;
    store.put_run(&stranded).await.unwrap();

    let seen = observer.get_run(stranded.run_id).await.unwrap();
    assert_eq!(seen.status, RunStatus::Failed, "an abandoned run must still be reaped");
    assert!(seen.error.is_some(), "and must say why");
}
