//! Readiness, which is a different question from liveness.
//!
//! `GET /ping` answers "this process is up", which is what ACP specifies and
//! what a supervisor deciding whether to restart wants. A load balancer is
//! deciding whether to *route*, and answering that with liveness gives a
//! replica whose store is unreachable a full share of traffic to fail.
//!
//! The store here is switched between reachable and not by a flag the test
//! owns, so "unready" is a state the test has established rather than an outage
//! it has to arrange.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusty_acp::server::store::{
    InMemoryStore, Notification, NotificationStream, RecoveryRecord, SessionRecord, Store,
    StoreResult,
};
use rusty_acp::server::{agent_fn, AcpServer, Readiness, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, Error, Event, Message, Run, RunId, Session, SessionId,
};

/// An [`InMemoryStore`] that can be cut off on demand.
///
/// Only `check_health` is affected: a store can be unreachable for readiness
/// purposes without every other call having to fail, and cutting off just the
/// probe keeps the test about what readiness reports.
#[derive(Debug, Default)]
struct FlakyStore {
    inner: InMemoryStore,
    reachable: AtomicBool,
    /// How many times the health check has actually run, so the cache can be
    /// asserted on rather than assumed.
    checks: AtomicUsize,
}

impl FlakyStore {
    fn new() -> Self {
        Self { reachable: AtomicBool::new(true), ..Self::default() }
    }

    fn cut_off(&self) {
        self.reachable.store(false, Ordering::SeqCst);
    }

    fn restore(&self) {
        self.reachable.store(true, Ordering::SeqCst);
    }

    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Store for FlakyStore {
    async fn check_health(&self) -> StoreResult<()> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if self.reachable.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Error::server_error("store ping failed: connection refused"))
        }
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
    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        self.inner.put_recovery_record(run_id, record).await
    }
    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        self.inner.recovery_record(run_id).await
    }
    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        self.inner.release_lease(run_id).await
    }
}

struct Replica {
    server: Arc<AcpServer>,
    base_url: String,
    store: Arc<FlakyStore>,
}

impl Replica {
    async fn new() -> Self {
        let store = Arc::new(FlakyStore::new());
        let echo = agent_fn(
            AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
            |ctx: RunContext| async move {
                ctx.reply_text(ctx.input_text()).await?;
                Ok(())
            },
        );

        let (server, router) = AcpServer::builder()
            .agent(echo)
            .store(Arc::clone(&store) as Arc<dyn Store>)
            .build()
            .unwrap()
            .into_shared_router();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        Self { server, base_url: format!("http://{addr}"), store }
    }

    async fn probe(&self, path: &str) -> (u16, serde_json::Value) {
        let response =
            reqwest::get(format!("{}{path}", self.base_url)).await.expect("the probe reaches it");
        let status = response.status().as_u16();
        let body = response.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    /// Let the readiness cache lapse, so the next probe asks the store again.
    async fn expire_cache(&self) {
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
}

#[tokio::test]
async fn a_healthy_replica_is_ready() {
    let replica = Replica::new().await;

    let (status, body) = replica.probe("/ready").await;

    assert_eq!(status, 200);
    assert_eq!(body["ready"], true);
    assert_eq!(body["accepting"], true);
    assert!(body.get("reason").is_none(), "a ready replica has nothing to explain: {body}");
}

/// The failure the endpoint exists for: a replica that answers `/ping`
/// perfectly while being unable to run anything.
#[tokio::test]
async fn an_unreachable_store_makes_a_replica_unready_but_still_alive() {
    let replica = Replica::new().await;
    replica.probe("/ready").await;
    replica.expire_cache().await;
    replica.store.cut_off();

    let (status, body) = replica.probe("/ready").await;
    assert_eq!(status, 503);
    assert_eq!(body["ready"], false);
    assert_eq!(body["reason"], "store_unreachable");
    assert!(body["detail"].as_str().is_some_and(|d| d.contains("connection refused")), "{body}");

    // Liveness is unmoved, and must be: the process is fine, and a supervisor
    // restarting it would throw away runs for a problem restarting cannot fix.
    let (ping, _) = replica.probe("/ping").await;
    assert_eq!(ping, 200);
}

#[tokio::test]
async fn readiness_recovers_when_the_store_does() {
    let replica = Replica::new().await;
    replica.store.cut_off();
    assert_eq!(replica.probe("/ready").await.0, 503);

    replica.store.restore();
    replica.expire_cache().await;

    assert_eq!(replica.probe("/ready").await.0, 200);
}

/// A draining replica must report unready *and* keep serving. That is the whole
/// reason the two signals are separate, and it is what stops a drain's 503s
/// from ever being sent — the balancer stops routing before they would be.
#[tokio::test]
async fn a_draining_replica_is_unready_while_still_serving() {
    let replica = Replica::new().await;
    replica.server.stop_accepting();

    let (status, body) = replica.probe("/ready").await;
    assert_eq!(status, 503);
    assert_eq!(body["reason"], "draining");
    assert_eq!(body["accepting"], false);

    // Still alive, and still answering reads for the runs it holds.
    assert_eq!(replica.probe("/ping").await.0, 200);
    assert_eq!(replica.probe("/agents").await.0, 200);
}

/// A drain is a local flag, so it shows up on the next probe rather than
/// whenever a cache happens to lapse — the cache is there to spare the store,
/// and a drain does not touch the store.
#[tokio::test]
async fn draining_is_reported_without_waiting_for_the_cache() {
    let replica = Replica::new().await;
    assert_eq!(replica.probe("/ready").await.0, 200);

    replica.server.stop_accepting();
    assert_eq!(replica.probe("/ready").await.0, 503, "the drain waited on a cache");
}

/// Being full is **not** unready.
///
/// A full replica is healthy and empties as its runs finish. Reporting it
/// unready would pull it from rotation under load, pushing its share onto
/// replicas that are also full — until a busy fleet has removed itself from
/// service entirely. A 429 sheds one request; an unready replica sheds all of
/// them.
#[tokio::test]
async fn a_replica_at_capacity_is_still_ready() {
    let forever = agent_fn(
        AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes"),
        |ctx: RunContext| async move {
            ctx.cancelled().await;
            Ok(())
        },
    );
    let (server, router) = AcpServer::builder()
        .agent(forever)
        .max_concurrent_runs(1)
        .build()
        .unwrap()
        .into_shared_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = rusty_acp::client::AcpClient::new(format!("http://{addr}")).unwrap();

    client.run_async("forever", [Message::user("go")]).await.unwrap();
    assert_eq!(server.executing(), 1);
    // Full: a second submission is refused.
    assert!(client.run_async("forever", [Message::user("go")]).await.is_err());

    assert!(server.readiness().await.is_ready(), "a busy replica removed itself from rotation");
    assert_eq!(reqwest::get(format!("http://{addr}/ready")).await.unwrap().status(), 200);
}

/// The cache is what keeps a probe schedule from becoming store load — most
/// sharply when the store is the thing already struggling.
#[tokio::test]
async fn the_store_is_not_probed_once_per_request() {
    let replica = Replica::new().await;

    for _ in 0..10 {
        assert_eq!(replica.probe("/ready").await.0, 200);
    }
    assert_eq!(
        replica.store.checks(),
        1,
        "ten probes became {} store round trips",
        replica.store.checks()
    );

    replica.expire_cache().await;
    assert_eq!(replica.probe("/ready").await.0, 200);
    assert_eq!(replica.store.checks(), 2, "the cache never lapsed");
}

/// The in-memory store cannot be unreachable — it is the process asking — so
/// the default implementation is the honest answer rather than a stub.
#[tokio::test]
async fn the_default_store_is_always_reachable() {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes"),
        |ctx: RunContext| async move { ctx.reply_text(ctx.input_text()).await.map(|_| ()) },
    );
    let server = AcpServer::builder().agent(echo).build().unwrap();

    assert_eq!(server.readiness().await, Readiness::Ready);
}
