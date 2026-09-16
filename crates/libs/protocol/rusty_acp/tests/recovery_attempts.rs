//! What a replacement run is charged for.
//!
//! The recovery ceiling exists to stop a run that *poisons* whatever executes
//! it: re-running it forever would migrate the same crash around the fleet. A
//! run whose replica walked away deliberately has demonstrated nothing of the
//! sort, and charging it means a rolling deploy across three replicas exhausts
//! the default budget in three hops — failing the run for something the agent
//! did not do.
//!
//! The decision under test is the reaper's, so these assert on the record the
//! *replacement* is given rather than on the marking that precedes it. A store
//! decorator captures every recovery record written, because the replacement's
//! run id is not otherwise knowable from outside.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::{Arc, Mutex};
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

/// Every recovery record written, in order.
type Written = Arc<Mutex<Vec<(RunId, RecoveryRecord)>>>;

/// An [`InMemoryStore`] that remembers the recovery records written through it.
#[derive(Debug)]
struct RecordingStore {
    inner: InMemoryStore,
    written: Written,
}

#[async_trait::async_trait]
impl Store for RecordingStore {
    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        if let Some(record) = record {
            self.written.lock().unwrap().push((run_id, record.clone()));
        }
        self.inner.put_recovery_record(run_id, record).await
    }

    async fn put_run(&self, run: &Run) -> StoreResult<()> {
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
    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        self.inner.release_lease(run_id).await
    }
}

/// Plant an abandoned run — non-terminal, no live lease — with the recovery
/// record under test, then read it through a replica so the reaper acts.
///
/// Returns the attempt the replacement was stamped with, if one was started.
async fn replacement_attempt_for(record: RecoveryRecord) -> Option<u32> {
    let written: Written = Arc::default();
    let store =
        Arc::new(RecordingStore { inner: InMemoryStore::default(), written: Arc::clone(&written) });

    let recoverable = agent_fn(
        AgentManifest::new(AgentName::new("replayable").unwrap(), "Can be re-run"),
        |ctx: RunContext| async move { ctx.reply_text("done").await.map(|_| ()) },
    )
    .with_recovery();

    let router = AcpServer::builder()
        .agent(recoverable)
        .store(Arc::clone(&store) as Arc<dyn Store>)
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = AcpClient::new(format!("http://{addr}")).unwrap();

    // What a replica that has gone away leaves behind.
    let mut abandoned = Run::new(AgentName::new("replayable").unwrap(), None);
    abandoned.status = RunStatus::InProgress;
    store.put_run(&abandoned).await.unwrap();
    store.put_recovery_record(abandoned.run_id, Some(&record)).await.unwrap();
    written.lock().unwrap().clear();

    // Reading it is what triggers the reap.
    client.get_run(abandoned.run_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The replacement is the only *other* run to be given a record.
    let written = written.lock().unwrap();
    written.iter().find(|(run_id, _)| *run_id != abandoned.run_id).map(|(_, record)| record.attempt)
}

/// A run whose replica died spends an attempt, as it always has.
#[tokio::test]
async fn a_death_still_spends_an_attempt() {
    let attempt = replacement_attempt_for(RecoveryRecord {
        input: vec![Message::user("go")],
        attempt: 1,
        handed_off: false,
    })
    .await;

    assert_eq!(attempt, Some(2), "the ceiling stopped counting deaths");
}

/// A run handed off by a draining replica does not.
#[tokio::test]
async fn a_hand_off_does_not() {
    let attempt = replacement_attempt_for(RecoveryRecord {
        input: vec![Message::user("go")],
        attempt: 1,
        handed_off: true,
    })
    .await;

    assert_eq!(attempt, Some(1), "a deploy was charged to the agent");
}

/// The replacement's own record starts clean, so a replacement that then dies
/// for real is charged normally. Without this, one hand-off would make a run
/// permanently exempt from the ceiling it exists to be caught by.
#[tokio::test]
async fn a_replacement_is_not_born_handed_off() {
    let written: Written = Arc::default();
    let store =
        Arc::new(RecordingStore { inner: InMemoryStore::default(), written: Arc::clone(&written) });

    let recoverable = agent_fn(
        AgentManifest::new(AgentName::new("replayable").unwrap(), "Can be re-run"),
        |ctx: RunContext| async move { ctx.reply_text("done").await.map(|_| ()) },
    )
    .with_recovery();

    let router = AcpServer::builder()
        .agent(recoverable)
        .store(Arc::clone(&store) as Arc<dyn Store>)
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = AcpClient::new(format!("http://{addr}")).unwrap();

    let mut abandoned = Run::new(AgentName::new("replayable").unwrap(), None);
    abandoned.status = RunStatus::InProgress;
    store.put_run(&abandoned).await.unwrap();
    store
        .put_recovery_record(
            abandoned.run_id,
            Some(&RecoveryRecord {
                input: vec![Message::user("go")],
                attempt: 1,
                handed_off: true,
            }),
        )
        .await
        .unwrap();
    written.lock().unwrap().clear();

    client.get_run(abandoned.run_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let written = written.lock().unwrap();
    let replacement = written
        .iter()
        .find(|(run_id, _)| *run_id != abandoned.run_id)
        .expect("a replacement was started");
    assert!(
        !replacement.1.handed_off,
        "a replacement inherited the exemption and can never be charged again"
    );
}
