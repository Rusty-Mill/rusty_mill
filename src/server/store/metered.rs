//! A [`Store`] decorator that times every operation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::server::telemetry;
use crate::{
    server::store::{
        Notification, NotificationStream, RecoveryRecord, SessionRecord, Store, StoreResult,
    },
    types::{Event, Message, Run, RunId, Session, SessionId},
};

/// Wraps any [`Store`], recording each operation's latency and failures.
///
/// Store calls are network I/O with a shared backend — every emit is a write
/// that can fail, which is why emitting returns `Result` at all. Their latency
/// and failure rate are the difference between "the fleet is slow" and "the
/// database is slow", and nothing else in the server can tell those apart.
///
/// # Opt-in, deliberately
///
/// Building a server does *not* wrap the store for you. Handing back something
/// other than what was passed in would be a surprising thing for a builder to
/// do — `server.store()` would no longer be the store the caller constructed,
/// and a backend author debugging their own implementation would be looking at
/// a decorator. Wrapping is one line where you can see it:
///
/// ```no_run
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::Arc;
/// use rusty_acp::server::store::{InMemoryStore, MeteredStore};
/// use rusty_acp::server::AcpServer;
/// # let my_agent = rusty_acp::server::agent_fn(
/// #     rusty_acp::types::AgentManifest::new(
/// #         rusty_acp::types::AgentName::new("echo")?, "Echoes"),
/// #     |ctx| async move { ctx.reply_text(ctx.input_text()).await?; Ok(()) });
///
/// let store = MeteredStore::new(Arc::new(InMemoryStore::default()));
///
/// let router = AcpServer::builder()
///     .agent(my_agent)
///     .store(Arc::new(store))
///     .build()?
///     .into_router();
/// # Ok(())
/// # }
/// ```
///
/// Operations are labelled by name — `put_run`, `append_event` and so on — never
/// by run id, which would be one time series per run.
#[derive(Debug)]
pub struct MeteredStore {
    inner: Arc<dyn Store>,
}

impl MeteredStore {
    /// Wrap `inner`, timing every operation.
    pub fn new(inner: Arc<dyn Store>) -> Self {
        Self { inner }
    }

    /// The store underneath.
    pub fn inner(&self) -> &Arc<dyn Store> {
        &self.inner
    }
}

/// Time an operation and record the outcome.
///
/// A macro rather than a function because the operations differ in return type
/// and in how many arguments they take; wrapping each by hand would be twenty
/// chances to record the wrong name.
macro_rules! timed {
    ($operation:literal, $call:expr) => {{
        let started = Instant::now();
        let result = $call.await;
        telemetry::store_operation($operation, started.elapsed(), result.is_err());
        result
    }};
}

#[async_trait::async_trait]
impl Store for MeteredStore {
    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        timed!("put_run", self.inner.put_run(run))
    }

    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        timed!("get_run", self.inner.get_run(run_id))
    }

    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
        timed!("append_event", self.inner.append_event(run_id, event))
    }

    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        timed!("events", self.inner.events(run_id))
    }

    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        timed!("events_from", self.inner.events_from(run_id, from))
    }

    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        timed!("publish", self.inner.publish(run_id, notification))
    }

    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        timed!("subscribe", self.inner.subscribe(run_id))
    }

    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        timed!("get_session", self.inner.get_session(session_id))
    }

    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        timed!("ensure_session", self.inner.ensure_session(session))
    }

    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        timed!(
            "append_session_messages",
            self.inner.append_session_messages(session_id, base_url, messages)
        )
    }

    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        timed!("get_session_state", self.inner.get_session_state(session_id))
    }

    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        timed!("put_session_state", self.inner.put_session_state(session_id, base_url, state))
    }

    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
        timed!("renew_lease", self.inner.renew_lease(run_id, owner, ttl))
    }

    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        timed!("lease_owner", self.inner.lease_owner(run_id))
    }

    async fn try_claim_lease(
        &self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> StoreResult<bool> {
        timed!("try_claim_lease", self.inner.try_claim_lease(run_id, owner, ttl))
    }

    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        timed!("put_recovery_record", self.inner.put_recovery_record(run_id, record))
    }

    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        timed!("recovery_record", self.inner.recovery_record(run_id))
    }

    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        timed!("release_lease", self.inner.release_lease(run_id))
    }
}
