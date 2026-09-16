//! What a drain has actually waited for when it returns.
//!
//! `drain` exists so a replica can be taken away without losing work, and the
//! only way to use it is to drain and then look at what is left. That makes the
//! store's view at the moment `drain` returns the whole contract: a run this
//! replica was executing must be *recorded* finished, not merely finished
//! executing.
//!
//! It was not. The execution slot lived in `RunContext`, so it was released
//! when the agent's body returned — ahead of the session write, the terminal
//! transition, the lease release and the recovery record being cleared. A drain
//! woken by that release read a run mid-flight and reported it as unfinished.
//!
//! **These tests do not race.** The window was four store writes wide, which on
//! the in-memory store made the misreport a coin flip — 89 times in 200 when
//! measured. A decorator that delays `put_run` by [`WRITE`] widens it to
//! something no scheduler can win, so a regression fails every run rather than
//! most of them.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{
    InMemoryStore, Notification, NotificationStream, RecoveryRecord, SessionRecord, Store,
    StoreResult,
};
use rusty_acp::server::{agent_fn, AcpServer, Drained, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, AwaitRequest, Event, Message, Run, RunId, RunStatus, Session,
    SessionId,
};

/// How long each run write takes. Long enough that a drain returning before one
/// lands is unmistakable, short enough to keep the suite quick.
const WRITE: Duration = Duration::from_millis(300);

/// An [`InMemoryStore`] whose run writes are slow.
///
/// `put_run` specifically, because that is the write the terminal transition
/// makes — the one a drain must not return in front of.
#[derive(Debug, Default)]
struct SlowWrites {
    inner: InMemoryStore,
}

#[async_trait::async_trait]
impl Store for SlowWrites {
    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        tokio::time::sleep(WRITE).await;
        self.inner.put_run(run).await
    }

    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        self.inner.get_run(run_id).await
    }
    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
        self.inner.append_event(run_id, event).await
    }
    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        self.inner.events(run_id).await
    }
    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        self.inner.events_from(run_id, from).await
    }
    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        self.inner.publish(run_id, notification).await
    }
    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        self.inner.subscribe(run_id).await
    }
    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        self.inner.get_session(session_id).await
    }
    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        self.inner.ensure_session(session).await
    }
    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        self.inner.append_session_messages(session_id, base_url, messages).await
    }
    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        self.inner.get_session_state(session_id).await
    }
    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        self.inner.put_session_state(session_id, base_url, state).await
    }
    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
        self.inner.renew_lease(run_id, owner, ttl).await
    }
    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        self.inner.lease_owner(run_id).await
    }
    async fn try_claim_lease(
        &self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> StoreResult<bool> {
        self.inner.try_claim_lease(run_id, owner, ttl).await
    }
    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        self.inner.recovery_record(run_id).await
    }
    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        self.inner.put_recovery_record(run_id, record).await
    }
    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        self.inner.release_lease(run_id).await
    }
}

/// A replica on a slow-writing store, and a client pointed at it.
async fn replica() -> (Arc<AcpServer>, Arc<dyn Store>, AcpClient) {
    let store: Arc<dyn Store> = Arc::new(SlowWrites::default());

    let quick = agent_fn(
        AgentManifest::new(AgentName::new("quick").unwrap(), "Returns straight away"),
        |ctx: RunContext| async move { ctx.reply_text("done").await.map(|_| ()) },
    );
    let asker = agent_fn(
        AgentManifest::new(AgentName::new("asker").unwrap(), "Parks awaiting an answer"),
        |ctx: RunContext| async move {
            ctx.await_request(AwaitRequest::new(serde_json::json!({ "q": "?" }))).await?;
            ctx.reply_text("answered").await.map(|_| ())
        },
    );

    let (server, router) = AcpServer::builder()
        .agent(quick)
        .agent(asker)
        .store(Arc::clone(&store))
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_shared_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client = AcpClient::new(format!("http://{addr}")).unwrap();
    (server, store, client)
}

/// The contract. A caller drains and then reads; what it reads must be the
/// finished run.
///
/// Without the fix the drain returns while the terminal `put_run` is still in
/// flight, and this reads `InProgress` — every time, not sometimes, because the
/// write it is racing takes 300ms.
#[tokio::test]
async fn a_drained_run_is_terminal_in_the_store() {
    let (server, store, client) = replica().await;
    let started = client.run_async("quick", [Message::user("go")]).await.unwrap();

    server.drain(Duration::from_secs(30)).await;

    let run = store.get_run(started.run_id).await.unwrap().expect("the run exists");
    assert_eq!(
        run.status,
        RunStatus::Completed,
        "drain returned with the run still {:?}; it waited for the agent, not for the run",
        run.status
    );
}

/// The same claim from the other side: the drain must be *slower* than the
/// writes it is waiting on. Guards against a fix that reported the right status
/// by luck of a read arriving late rather than by holding the slot.
///
/// Two writes, not one. A run makes two `put_run` calls after it is created —
/// `set_in_progress` before the agent body and the terminal transition after it
/// — and waiting only for the agent body already covers the first. One `WRITE`
/// would therefore pass without the fix, measuring nothing; the second write is
/// the whole of what changed.
///
/// The threshold sits *between* the two outcomes rather than on either. Without
/// the fix a drain returns at about one `WRITE`, with it at about two, and the
/// clock starts after `run_async` has returned — by which point the first write
/// is already partly done. Asserting `>= 2 * WRITE` measured 598.96ms against a
/// 600ms bar and failed on the millisecond, which is a flake rather than a
/// finding.
#[tokio::test]
async fn a_drain_outlasts_the_writes_it_waits_on() {
    let (server, _store, client) = replica().await;
    client.run_async("quick", [Message::user("go")]).await.unwrap();

    let began = Instant::now();
    server.drain(Duration::from_secs(30)).await;

    assert!(
        began.elapsed() > WRITE + WRITE / 2,
        "drain returned in {:?}, which is not long enough to be behind the terminal write",
        began.elapsed()
    );
}

/// The symptom that surfaced this: a finished run named as unfinished.
///
/// Needs a parked run present, because `drain` short-circuits to an empty
/// `Drained` when nothing is parked and the deadline was met — which is exactly
/// why no existing test caught this.
#[tokio::test]
async fn a_finished_run_is_not_reported_as_unfinished() {
    let (server, store, client) = replica().await;
    client.run_sync("asker", [Message::user("hi")]).await.unwrap();
    let running = client.run_async("quick", [Message::user("go")]).await.unwrap();

    let Drained { unfinished, parked } = server.drain(Duration::from_secs(30)).await;

    assert_eq!(parked.len(), 1, "the conversation should be handed back");
    assert!(
        !unfinished.contains(&running.run_id),
        "a run that completed was handed back as unfinished work"
    );
    for run_id in &unfinished {
        let run = store.get_run(*run_id).await.unwrap().expect("the run exists");
        assert!(
            !matches!(run.status, RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled),
            "{run_id} was reported unfinished but is {:?}",
            run.status
        );
    }
}

/// Holding the slot to the end must not resurrect the bug #44 fixed: a parked
/// conversation gives its capacity back, so a drain never waits for a client
/// who is not answering.
#[tokio::test]
async fn a_parked_conversation_still_does_not_hold_the_drain() {
    let (server, _store, client) = replica().await;
    client.run_sync("asker", [Message::user("hi")]).await.unwrap();

    let began = Instant::now();
    let drained = server.drain(Duration::from_secs(30)).await;

    assert_eq!(drained.parked.len(), 1);
    assert!(
        began.elapsed() < Duration::from_secs(30),
        "the drain sat out its deadline for a parked conversation"
    );
}
