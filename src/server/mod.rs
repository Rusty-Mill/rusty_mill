//! Host your own agents behind the standard ACP HTTP endpoints.
//!
//! Implement [`Agent`], register it with [`AcpServer::builder`], and serve the
//! resulting [`axum::Router`]. The server implements the whole protocol
//! surface: discovery, the three run modes, await/resume, cancellation, the
//! event log and distributed sessions.
//!
//! ```no_run
//! use rusty_acp::server::{agent_fn, AcpServer};
//! use rusty_acp::types::{AgentManifest, AgentName};
//!
//! # async fn serve() -> Result<(), Box<dyn std::error::Error>> {
//! let echo = agent_fn(
//!     AgentManifest::new(AgentName::new("echo")?, "Echoes the input back"),
//!     |ctx| async move {
//!         ctx.reply_text(ctx.input_text()).await?;
//!         Ok(())
//!     },
//! );
//!
//! let router = AcpServer::builder().agent(echo).build()?.into_router();
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
//! axum::serve(listener, router).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Running several replicas
//!
//! By default runs live in process memory, which is right for a single agent
//! host. To put several replicas behind a load balancer, give each the *same*
//! shared [`Store`] — then any replica can serve any request for
//! any run, and no session affinity is needed:
//!
//! ```no_run
//! # #[cfg(feature = "redis-store")]
//! # async fn serve() -> Result<(), Box<dyn std::error::Error>> {
//! # use rusty_acp::server::{AcpServer, store::RedisStore};
//! # let my_agent = rusty_acp::server::agent_fn(
//! #     rusty_acp::types::AgentManifest::new(
//! #         rusty_acp::types::AgentName::new("echo")?, "Echoes"),
//! #     |ctx| async move { ctx.reply_text(ctx.input_text()).await?; Ok(()) });
//! let store = RedisStore::connect("redis://127.0.0.1/").await?;
//!
//! let router = AcpServer::builder()
//!     .agent(my_agent)
//!     .store(std::sync::Arc::new(store))
//!     .build()?
//!     .into_router();
//! # Ok(())
//! # }
//! ```
//!
//! If a replica dies mid-run, the run it was executing would otherwise stay
//! non-terminal forever. Each executing replica holds a renewed lease on its
//! runs, so a non-terminal run whose lease has lapsed is recognised as
//! abandoned and failed by whichever replica next reads it — see
//! [`AcpServerBuilder::lease_ttl`] and [`Store::renew_lease`].
//!
//! # Recovering a lost run
//!
//! Failing an abandoned run is always correct but unambitious: the work is
//! lost and the client resubmits. An agent that declares itself
//! [`recoverable`](Agent::recoverable) gets more — when its replica dies, the
//! server starts a **replacement run** with the same input and session and a
//! fresh id, and links the two:
//!
//! ```text
//! run A: failed   error.data.replaced_by = <run B>
//!    └── run B: running   generic event { replaces: <run A>, attempt: 2 }
//! ```
//!
//! The abandoned run keeps its own history and stays failed. Nothing already
//! streamed to a client is retracted, and no run ever ends up with two sets of
//! output — which is why this is a new run rather than a re-execution in place.
//!
//! Three things are worth knowing before opting in:
//!
//! - **The default is `false`, deliberately.** Replaying an agent that takes a
//!   payment or sends a message repeats it. ACP carries no idempotency
//!   contract, so the server cannot infer which agents are safe.
//! - **Every replica must host the same agents.** The replica that notices an
//!   abandoned run is the one that re-runs it; if it does not have that agent
//!   registered, the run is failed as usual.
//! - **There is an attempt ceiling** — see
//!   [`AcpServerBuilder::max_recovery_attempts`] — so a run that kills whatever
//!   executes it cannot migrate around the fleet forever.
//!
//! See the [`store`] module for what a backend must guarantee.

mod agent;
mod routes;
mod run;
pub mod store;
mod telemetry;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{http::HeaderMap, Router};
use tokio::sync::watch;
use tracing::Instrument;

use crate::types::{
    AgentManifest, AgentName, Error, Event, Message, Run, RunCreateRequest, RunId, RunStatus,
    Session, SessionId,
};

pub use agent::{agent_fn, Agent, FnAgent, MessageWriter, RunContext};
pub use run::RunHandle;
pub use store::{
    InMemoryStore, Notification, RecoveryRecord, SessionRecord, Store, DEFAULT_MAX_RUNS,
    DEFAULT_MAX_SESSIONS,
};

#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
pub use store::{RedisStore, RedisStoreConfig};

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub use store::MeteredStore;

use store::NotificationStream;

/// The default base URL used to build session history links when none is
/// configured and the request carries no `Host` header.
const DEFAULT_BASE_URL: &str = "http://localhost:8000";

/// How long a run's ownership lease survives without renewal.
///
/// A replica that stops renewing for this long is treated as gone and its runs
/// are failed. Renewal happens three times per lease lifetime, so several
/// renewals have to be missed before the lease lapses and a slow tick or brief
/// pause is not mistaken for death.
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);

/// The lease is renewed every `ttl / LEASE_RENEW_DIVISOR`, so several renewals
/// have to be missed before the lease lapses.
const LEASE_RENEW_DIVISOR: u32 = 3;

/// How long a `sync` request waits before returning whatever the run has
/// reached so far.
///
/// A `sync` call that never returns is worse than one that returns an
/// unfinished snapshot: proxies and load balancers cut the connection anyway,
/// and the caller is left with nothing to act on.
pub const DEFAULT_SYNC_TIMEOUT: Duration = Duration::from_secs(300);

/// How many times a run may be started before recovery gives up on it.
///
/// Counts attempts, not retries: `1` disables recovery entirely. Without a
/// ceiling, a run that reliably kills whatever executes it would migrate around
/// the fleet forever.
pub const DEFAULT_MAX_RECOVERY_ATTEMPTS: u32 = 3;

/// How long a readiness answer is reused before the store is asked again.
///
/// A load balancer probes on a schedule, from every replica, forever. Without a
/// cache that becomes a store round trip per probe per replica — load the store
/// pays most heavily exactly when it is already the thing struggling, which is
/// the one moment readiness has to keep working.
///
/// Short enough that a store coming back is noticed within a probe interval,
/// which is what the cache costs: recovery is seen up to this late.
///
/// Public because it is not configurable and an operator choosing a probe
/// interval has to know it — a probe faster than this reads a cached answer,
/// which is the intent, but only if you know that is what is happening.
pub const READINESS_CACHE: Duration = Duration::from_secs(1);

/// What a drain could not finish, split by why.
///
/// The two are different situations and an operator reading a shutdown log
/// wants to tell them apart: unfinished runs mean the deadline was too short
/// for the work, parked ones mean clients were mid-conversation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Drained {
    /// Runs still executing an agent body when the deadline passed.
    pub unfinished: Vec<RunId>,
    /// Runs parked awaiting a client answer.
    ///
    /// **These cannot survive this replica.** An agent that has paused to ask a
    /// question is suspended part-way through its own function, and that
    /// position lives in this process — no other replica can resume from it.
    /// A `recoverable` agent gets a replacement started from its input, which
    /// re-asks the question; anything else is failed. What the drain can do,
    /// and does, is make that happen *now* rather than after the lease lapses,
    /// and without waiting on a client who may never answer.
    pub parked: Vec<RunId>,
}

impl Drained {
    /// Whether the replica finished everything it was holding.
    pub fn is_empty(&self) -> bool {
        self.unfinished.is_empty() && self.parked.is_empty()
    }

    /// How many runs were handed back, of either kind.
    pub fn len(&self) -> usize {
        self.unfinished.len() + self.parked.len()
    }

    /// Every run handed back, unfinished first.
    pub fn run_ids(&self) -> impl Iterator<Item = RunId> + '_ {
        self.unfinished.iter().chain(&self.parked).copied()
    }
}

/// Whether this replica should be sent new work, and why not if not.
///
/// Distinct from liveness. `GET /ping` answers "this process is up", which is
/// what ACP specifies and what a restart-on-failure supervisor wants. This
/// answers "send me traffic", which is a different question with different
/// right answers — a draining replica is emphatically alive and must keep
/// serving what it already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Send work.
    Ready,
    /// Alive, still serving in-flight runs, but on its way out.
    Draining,
    /// The store cannot be reached, so any run started here would fail.
    StoreUnreachable(String),
}

impl Readiness {
    /// Whether new work should be routed here.
    pub fn is_ready(&self) -> bool {
        matches!(self, Readiness::Ready)
    }

    /// A short machine-readable reason, or `None` when ready.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Readiness::Ready => None,
            Readiness::Draining => Some("draining"),
            Readiness::StoreUnreachable(_) => Some("store_unreachable"),
        }
    }
}

/// How long a run may sit `awaiting` a client answer before it is failed.
///
/// A parked run is not free. It holds a task, an entry the default store will
/// never evict because active runs are never evicted, and a lease its replica
/// keeps renewing — a store write every `lease_ttl / 3`, forever, for a
/// conversation nobody is having. Reachable by anyone who can submit a run:
/// ask a question, never answer, repeat.
///
/// An hour is deliberately generous. A human-in-the-loop agent waiting on an
/// actual human may legitimately park for a long time, and the cost of cutting
/// one of those off early is a failed run the client can resubmit — against the
/// cost of being wrong the other way, which is unbounded growth. Set it to
/// taste, or switch it off with
/// [`without_await_timeout`](AcpServerBuilder::without_await_timeout) if your
/// conversations really are open-ended.
pub const DEFAULT_AWAIT_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// How long [`AcpServer::shutdown`] waits for runs in flight by default.
///
/// Generous, because the cost of the two failure modes is not symmetric. Too
/// short and a deploy discards work that was seconds from finishing; too long
/// and the deploy takes longer, which an orchestrator's own termination grace
/// period bounds anyway.
pub const DEFAULT_DRAIN_DEADLINE: Duration = Duration::from_secs(60);

/// The runs this replica is executing, and whether it will take any more.
///
/// Leases exist because a replica can *die* without warning, and nothing can be
/// done about that from inside the dying process. A replica being deployed over
/// is a different situation wearing the same clothes: it knows it is going
/// away, and it has time to act. Tracking what is in flight is what lets it use
/// that knowledge instead of throwing it away and letting every rolling deploy
/// look like a crash.
///
/// It also holds this replica's execution capacity. The two belong together:
/// both answer "should this replica take on this run", differing only in why
/// the answer might be no.
#[derive(Debug)]
pub(crate) struct InFlight {
    /// Whether new runs still start here.
    accepting: AtomicBool,
    /// How many runs may execute at once, if a ceiling was set.
    limit: Option<usize>,
    /// Runs holding an execution slot right now.
    ///
    /// Not the same as `running.len()`: a run parked awaiting a client answer
    /// is still this replica's to finish, but is costing it a suspended future
    /// and nothing more.
    executing: Mutex<usize>,
    /// Runs this replica owns, so the ones a drain leaves behind can be found
    /// and have their leases released.
    running: Mutex<HashSet<RunId>>,
    /// The subset of `running` parked awaiting a client answer.
    ///
    /// Tracked separately because a drain must treat them differently: they are
    /// not going to finish, however long it waits.
    parked: Mutex<HashSet<RunId>>,
    /// The count of runs holding an execution slot, published so a drain can
    /// await zero rather than poll for it — a polled drain would either add
    /// latency to every deploy or burn CPU waiting.
    ///
    /// The *executing* count rather than the size of `running`, which is the
    /// whole of the fix for a drain that used to wait on parked conversations.
    idle: watch::Sender<usize>,
}

impl InFlight {
    fn new(limit: Option<usize>) -> Self {
        Self {
            accepting: AtomicBool::new(true),
            limit,
            executing: Mutex::new(0),
            running: Mutex::new(HashSet::new()),
            parked: Mutex::new(HashSet::new()),
            idle: watch::channel(0).0,
        }
    }

    /// Record the executing count and publish it, under the lock that owns it.
    ///
    /// One place, so the gauge, the admission check and the drain can never
    /// disagree about how many runs are actually running.
    fn publish(&self, executing: &usize) {
        telemetry::runs_executing(*executing);
        self.idle.send_replace(*executing);
    }

    /// Take a slot for a new run, if this replica has one to give.
    ///
    /// `None` means at capacity, and the caller's job is to say so rather than
    /// to queue. An unbounded queue is the same failure with a longer fuse, and
    /// a bounded one is a second capacity number to tune for nothing the
    /// ceiling does not already provide — the client can wait far more cheaply
    /// than the server can hold the request open on its behalf.
    fn admit(self: &Arc<Self>) -> Option<Slot> {
        let mut executing = self.executing.lock().expect("capacity counter poisoned");
        if self.limit.is_some_and(|limit| *executing >= limit) {
            return None;
        }
        *executing += 1;
        self.publish(&executing);
        Some(Slot { capacity: Arc::clone(self), held: AtomicBool::new(true) })
    }

    /// Take a slot for work the fleet has already accepted, over the ceiling if
    /// need be.
    ///
    /// Used for a replacement run. Refusing one would not defer the work, it
    /// would lose the run — recovery has nobody to retry it, unlike a client
    /// meeting a 429.
    fn take(self: &Arc<Self>) -> Slot {
        let mut executing = self.executing.lock().expect("capacity counter poisoned");
        *executing += 1;
        self.publish(&executing);
        drop(executing);
        Slot { capacity: Arc::clone(self), held: AtomicBool::new(true) }
    }

    fn release(&self) {
        let mut executing = self.executing.lock().expect("capacity counter poisoned");
        *executing = executing.saturating_sub(1);
        self.publish(&executing);
    }

    fn reacquire(&self) {
        let mut executing = self.executing.lock().expect("capacity counter poisoned");
        *executing += 1;
        self.publish(&executing);
    }

    /// This run is now waiting on a client rather than running.
    fn park(&self, run_id: RunId) {
        self.parked.lock().expect("parked set poisoned").insert(run_id);
    }

    /// The client answered; it is running again.
    fn unpark(&self, run_id: RunId) {
        self.parked.lock().expect("parked set poisoned").remove(&run_id);
    }

    fn executing(&self) -> usize {
        *self.executing.lock().expect("capacity counter poisoned")
    }

    fn enter(&self, run_id: RunId) {
        self.running.lock().expect("in-flight set poisoned").insert(run_id);
    }

    fn leave(&self, run_id: RunId) {
        self.running.lock().expect("in-flight set poisoned").remove(&run_id);
        // Also here, not only in `unpark`: a run cancelled while parked never
        // unparks, and leaving it in the set would have a drain report a
        // conversation that has already ended.
        self.parked.lock().expect("parked set poisoned").remove(&run_id);
    }

    /// What this replica is holding, split by whether it can still finish.
    fn snapshot(&self) -> (Vec<RunId>, Vec<RunId>) {
        let running = self.running.lock().expect("in-flight set poisoned");
        let parked = self.parked.lock().expect("parked set poisoned");
        let unfinished = running.difference(&parked).copied().collect();
        (unfinished, parked.iter().copied().collect())
    }

    fn len(&self) -> usize {
        self.running.lock().expect("in-flight set poisoned").len()
    }

    /// Resolve once no run is executing an agent body here.
    ///
    /// Deliberately *not* "once nothing is left": a run parked awaiting a
    /// client is not going to finish however long this waits, so counting it
    /// would make every drain sit out its whole deadline for a conversation
    /// that is doing nothing.
    async fn idle(&self) {
        // `wait_for` tests the current value before waiting, so a replica that
        // is already idle returns immediately rather than blocking on a change
        // that will never come.
        let _ = self.idle.subscribe().wait_for(|executing| *executing == 0).await;
    }
}

/// One unit of this replica's execution capacity, held while a run is actually
/// running — from admission until its outcome is recorded, not merely until
/// the agent's body returns.
///
/// That distinction is the whole of #54. The slot is shared between the run's
/// [`RunContext`] and the task executing it, so releasing it is the *last*
/// thing that happens rather than something the agent's own future does on its
/// way out. A drain waits on this capacity, and a drain that returns before the
/// run's terminal write has landed has not waited for anything a caller can
/// read.
///
/// Released while the run is parked awaiting a client answer, and taken again
/// when the answer arrives. That asymmetry is the point of the type: an
/// `awaiting` run is waiting on a human who may never come back, and counting
/// it against capacity would let idle conversations starve live work.
///
/// Reacquiring is deliberately unchecked. The run was admitted once already,
/// and refusing it here would strand a conversation mid-sentence to protect a
/// ceiling — so a burst of resumes can briefly exceed the limit. The limit
/// governs what this replica *takes on*, not an instantaneous invariant.
#[derive(Debug)]
pub(crate) struct Slot {
    capacity: Arc<InFlight>,
    held: AtomicBool,
}

impl Slot {
    /// Give the slot up while the run waits for a client.
    pub(crate) fn park(&self, run_id: RunId) {
        if self.held.swap(false, Ordering::SeqCst) {
            self.capacity.park(run_id);
            self.capacity.release();
        }
    }

    /// Take it back now the client has answered.
    pub(crate) fn unpark(&self, run_id: RunId) {
        if !self.held.swap(true, Ordering::SeqCst) {
            self.capacity.unpark(run_id);
            self.capacity.reacquire();
        }
    }
}

impl Drop for Slot {
    /// Return the capacity, without recording a park.
    ///
    /// A dropped slot belongs to a run that is ending, not one that is waiting
    /// — `InFlight::leave` clears it from both sets. Marking it parked here
    /// would have a drain report a conversation that has already finished.
    ///
    /// Runs when the last reference goes, which is the spawning task's, after
    /// it has left the in-flight set. A run parked when it ended — cancelled
    /// mid-conversation — already gave the capacity back, and `held` is what
    /// stops this returning it twice.
    fn drop(&mut self) {
        if self.held.swap(false, Ordering::SeqCst) {
            self.capacity.release();
        }
    }
}

/// A configured ACP server: a set of agents plus the store backing their runs.
///
/// Build one with [`AcpServer::builder`], then call
/// [`into_router`](AcpServer::into_router).
#[derive(Debug)]
pub struct AcpServer {
    agents: HashMap<AgentName, Arc<dyn Agent>>,
    order: Vec<AgentName>,
    store: Arc<dyn Store>,
    base_url: Option<String>,
    replica_id: String,
    lease_ttl: Duration,
    sync_timeout: Option<Duration>,
    max_recovery_attempts: u32,
    await_timeout: Option<Duration>,
    in_flight: Arc<InFlight>,
    /// The last readiness answer and when it was given.
    readiness: Mutex<Option<(tokio::time::Instant, Readiness)>>,
}

impl std::fmt::Debug for dyn Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent").field("name", &self.manifest().name).finish()
    }
}

impl AcpServer {
    /// Start configuring a server.
    pub fn builder() -> AcpServerBuilder {
        AcpServerBuilder::default()
    }

    /// Consume the server, producing the router that serves the ACP endpoints.
    pub fn into_router(self) -> Router {
        routes::router(Arc::new(self))
    }

    /// Produce the router while keeping a handle on the server, so runs and
    /// sessions can be inspected from outside the HTTP layer.
    pub fn into_shared_router(self) -> (Arc<Self>, Router) {
        let server = Arc::new(self);
        (Arc::clone(&server), routes::router(Arc::clone(&server)))
    }

    /// The store backing runs and sessions.
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// This replica's identifier, used as the owner on run leases.
    pub fn replica_id(&self) -> &str {
        &self.replica_id
    }

    /// How long this replica's run leases survive without renewal.
    pub fn lease_ttl(&self) -> Duration {
        self.lease_ttl
    }

    /// How long a `sync` request waits before returning the run as it stands.
    pub fn sync_timeout(&self) -> Option<Duration> {
        self.sync_timeout
    }

    /// How many times a run may be started before recovery gives up.
    pub fn max_recovery_attempts(&self) -> u32 {
        self.max_recovery_attempts
    }

    /// How long a run may sit `awaiting` before it is failed, if bounded.
    pub fn await_timeout(&self) -> Option<Duration> {
        self.await_timeout
    }

    /// How many runs may execute here at once, if a ceiling was set.
    pub fn max_concurrent_runs(&self) -> Option<usize> {
        self.in_flight.limit
    }

    /// How many runs are running an agent body right now.
    ///
    /// Excludes runs parked awaiting a client answer, which is what
    /// [`max_concurrent_runs`](AcpServer::max_concurrent_runs) is measured
    /// against.
    pub fn executing(&self) -> usize {
        self.in_flight.executing()
    }

    /// Whether this replica is still starting new runs.
    ///
    /// False from [`stop_accepting`](AcpServer::stop_accepting) onwards. Worth
    /// exposing because it is what a readiness probe should report: a draining
    /// replica is still serving reads, cancellations and the runs it already
    /// has, and should keep receiving that traffic — it just must not be sent
    /// any more work.
    pub fn is_accepting(&self) -> bool {
        self.in_flight.accepting.load(Ordering::SeqCst)
    }

    /// How many runs this replica is executing right now.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Take an execution slot for a new run, if this replica has one.
    ///
    /// `None` means at capacity. The caller answers 429 rather than waiting:
    /// holding the request open would spend a connection and a task on exactly
    /// the replica that has none to spare.
    pub(crate) fn admit(&self) -> Option<Slot> {
        self.in_flight.admit()
    }

    /// Whether this replica should be sent new work.
    ///
    /// Answers the question a load balancer's readiness probe asks, which is
    /// not the one `GET /ping` answers. A draining replica is alive and must
    /// keep serving the runs it already has; it just must not be given more.
    ///
    /// **Being at capacity is deliberately not unready.** A full replica is
    /// healthy and empties as its runs finish. Reporting it unready would pull
    /// it out of rotation under load — pushing its share onto replicas that are
    /// also full, which report unready in turn, until a busy fleet has removed
    /// itself from service. A 429 sheds one request; an unready replica sheds
    /// all of them, and the difference is a bad afternoon versus an outage.
    ///
    /// The store check is cached for [`READINESS_CACHE`], so probing this in a
    /// tight loop does not become load on the store.
    pub async fn readiness(&self) -> Readiness {
        if !self.is_accepting() {
            // Not cached: this is a local flag, and a drain should be visible
            // to the next probe rather than up to a cache-interval later.
            return Readiness::Draining;
        }

        if let Some(cached) = self.cached_readiness() {
            return cached;
        }

        let readiness = match self.store.check_health().await {
            Ok(()) => Readiness::Ready,
            Err(error) => {
                tracing::warn!(%error, "readiness: the store is unreachable");
                Readiness::StoreUnreachable(error.to_string())
            }
        };
        *self.readiness.lock().expect("readiness cache poisoned") =
            Some((tokio::time::Instant::now(), readiness.clone()));
        readiness
    }

    fn cached_readiness(&self) -> Option<Readiness> {
        let cached = self.readiness.lock().expect("readiness cache poisoned");
        let (asked, readiness) = cached.as_ref()?;
        (asked.elapsed() < READINESS_CACHE).then(|| readiness.clone())
    }

    /// Stop starting new runs here.
    ///
    /// Takes effect before it returns: `POST /runs` answers 503 with a
    /// `Retry-After` from the next request on, and this replica stops adopting
    /// abandoned runs it comes across. Idempotent, and not reversible — a
    /// replica that has begun going away does not come back.
    ///
    /// Kept separate from [`drain`](AcpServer::drain) deliberately. A
    /// deployment wants to stop taking work, tell its load balancer, and *then*
    /// wait — and the waiting is the long part. Folding the two together would
    /// mean the balancer only found out once the drain was over, having spent
    /// the whole drain sending requests that were refused.
    pub fn stop_accepting(&self) {
        if !self.in_flight.accepting.swap(false, Ordering::SeqCst) {
            return;
        }
        tracing::info!(
            replica = %self.replica_id,
            in_flight = self.in_flight.len(),
            "no longer accepting new runs"
        );
    }

    /// Wait for the runs in flight to finish, for up to `deadline`.
    ///
    /// Returns the runs that were still going when the deadline passed; an
    /// empty result means the replica drained cleanly and can exit having
    /// finished everything it was given.
    ///
    /// **A run that outlasts the deadline has its lease released rather than
    /// being failed here.** Failing it would end a run that was seconds from
    /// finishing, and this replica is in no position to judge — it is leaving.
    /// Releasing hands the decision to whoever picks the run up: a recoverable
    /// agent gets a replacement started immediately, and everything else is
    /// failed by the next replica to read it. Both already work; the release is
    /// what makes them happen *now* instead of after the lease would have
    /// lapsed on its own.
    ///
    /// Call [`stop_accepting`](AcpServer::stop_accepting) first, or new runs
    /// will keep arriving and the drain may never finish.
    pub async fn drain(&self, deadline: Duration) -> Drained {
        let finished = tokio::time::timeout(deadline, self.in_flight.idle()).await.is_ok();
        let (unfinished, parked) = self.in_flight.snapshot();

        if finished && parked.is_empty() {
            tracing::info!(replica = %self.replica_id, "drained cleanly");
            return Drained::default();
        }
        if !finished {
            tracing::warn!(
                replica = %self.replica_id,
                count = unfinished.len(),
                ?deadline,
                "drain deadline passed with runs still executing"
            );
        }
        if !parked.is_empty() {
            tracing::info!(
                replica = %self.replica_id,
                count = parked.len(),
                "handing back conversations parked awaiting a client"
            );
        }

        for run_id in unfinished.iter().chain(&parked) {
            // Marked *before* the lease is released, not after. Releasing is
            // what lets another replica claim the run, and it can do so
            // immediately — the same write-before-signal rule the cancellation
            // path learned in #27. Marked after, a fast reaper reads the old
            // record and charges the agent for a deploy.
            self.mark_handed_off(*run_id).await;

            if let Err(error) = self.store.release_lease(*run_id).await {
                // Warn, unlike the release on a run's normal completion. There
                // the lease was a formality on an already-terminal run; here it
                // is the signal that this run needs a new owner, and losing it
                // costs a client the whole lease TTL of waiting on a run nobody
                // is executing.
                tracing::warn!(%run_id, %error, "failed to release a drained run's lease");
            }
        }
        Drained { unfinished, parked }
    }

    /// Record that this run is being given up deliberately.
    ///
    /// Only touches runs that have a recovery record — everything else is
    /// failed by whoever picks it up, and failing spends no attempts. A failure
    /// here is logged rather than propagated: the worst outcome is that the
    /// replacement costs an attempt it should not have, which is the behaviour
    /// this replaced and is far better than a shutdown that stalls.
    async fn mark_handed_off(&self, run_id: RunId) {
        let record = match self.store.recovery_record(run_id).await {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%run_id, %error, "failed to read a recovery record while draining");
                return;
            }
        };
        if record.handed_off {
            return;
        }
        let marked = RecoveryRecord { handed_off: true, ..record };
        if let Err(error) = self.store.put_recovery_record(run_id, Some(&marked)).await {
            tracing::warn!(%run_id, %error, "failed to mark a run as handed off while draining");
        }
    }

    /// Stop accepting new runs, then wait for the ones in flight.
    ///
    /// The two steps in the order a shutdown wants them. Use them separately if
    /// there is something to do in between — telling a load balancer, most
    /// obviously.
    ///
    /// ```no_run
    /// # use std::time::Duration;
    /// # use rusty_acp::server::{AcpServer, DEFAULT_DRAIN_DEADLINE};
    /// # async fn demo(server: std::sync::Arc<AcpServer>) {
    /// let drained = server.shutdown(DEFAULT_DRAIN_DEADLINE).await;
    /// if !drained.is_empty() {
    ///     eprintln!(
    ///         "{} unfinished, {} parked mid-conversation",
    ///         drained.unfinished.len(),
    ///         drained.parked.len(),
    ///     );
    /// }
    /// # }
    /// ```
    pub async fn shutdown(&self, deadline: Duration) -> Drained {
        self.stop_accepting();
        self.drain(deadline).await
    }

    /// Manifests of every registered agent, in registration order.
    pub fn manifests(&self) -> Vec<AgentManifest> {
        self.order.iter().filter_map(|name| self.manifest(name)).collect()
    }

    /// The manifest of one agent.
    pub fn manifest(&self, name: &AgentName) -> Option<AgentManifest> {
        self.agents.get(name).map(|agent| agent.manifest())
    }

    /// The configured base URL, if one was set.
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// The base URL to use for links in a response.
    ///
    /// Prefers the explicitly configured base URL, then the request's `Host`
    /// header (honouring `X-Forwarded-Proto` and `X-Forwarded-Host`), and
    /// finally [`DEFAULT_BASE_URL`].
    ///
    /// With several replicas, set the base URL explicitly to the load
    /// balancer's address: a session's history links must stay resolvable no
    /// matter which replica wrote them.
    fn resolve_base_url(&self, headers: &HeaderMap) -> String {
        if let Some(base_url) = &self.base_url {
            return base_url.trim_end_matches('/').to_string();
        }
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let host = header("x-forwarded-host").or_else(|| header("host"));
        match host {
            Some(host) => {
                let scheme = header("x-forwarded-proto").unwrap_or("http");
                format!("{scheme}://{host}")
            }
            None => DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Validate a create request, register the run, and spawn its agent.
    ///
    /// Returns the run id and a notification stream subscribed *before* the
    /// run emitted anything, so the caller sees every event from `run.created`
    /// onwards.
    async fn start_run(
        self: &Arc<Self>,
        slot: Slot,
        request: RunCreateRequest,
        base_url: &str,
    ) -> Result<(RunId, NotificationStream), Error> {
        let agent =
            self.agents.get(&request.agent_name).cloned().ok_or_else(|| {
                Error::not_found(format!("agent {} not found", request.agent_name))
            })?;

        check_input_content_types(&agent.manifest(), &request.input)?;

        // Resolve the session before the run, so history is captured as it
        // stood before this run's input was added.
        let (session, history) = self.resolve_session(&request).await?;

        self.launch(
            LaunchSpec {
                slot,
                agent,
                agent_name: request.agent_name,
                input: request.input,
                session,
                history,
                attempt: 1,
                // A first attempt contributes its input to the session; a
                // replacement must not, since the run it replaces already did.
                record_input_in_session: true,
                replaces: None,
            },
            base_url,
        )
        .await
    }

    /// Create a run, take its lease, and spawn its agent.
    ///
    /// Shared by fresh runs and by the replacements created when a run is
    /// recovered, so both go through exactly the same setup.
    async fn launch(
        self: &Arc<Self>,
        spec: LaunchSpec,
        base_url: &str,
    ) -> Result<(RunId, NotificationStream), Error> {
        let LaunchSpec {
            slot,
            agent,
            agent_name,
            input,
            session,
            history,
            attempt,
            record_input_in_session,
            replaces,
        } = spec;

        let session_id = session.as_ref().map(|session| session.id);
        let run = Run::new(agent_name.clone(), session_id);
        let run_id = run.run_id;

        // Take ownership *before* the run exists in the store. A run that is
        // visible but unowned would look abandoned, and a reaper could fail it
        // before it ever started.
        self.store.renew_lease(run_id, &self.replica_id, self.lease_ttl).await?;

        // Only recoverable agents get their input persisted. For everyone else
        // the absence of a record is what makes fail-fast the default.
        if agent.recoverable() {
            self.store
                .put_recovery_record(
                    run_id,
                    Some(&RecoveryRecord { input: input.clone(), attempt, handed_off: false }),
                )
                .await?;
        }

        self.store.put_run(&run).await?;

        // Subscribe before emitting anything so a streaming client sees
        // `run.created` and everything after it.
        let client_stream = self.store.subscribe(run_id).await?;
        let control_stream = self.store.subscribe(run_id).await?;

        // Before `run.created` goes out, for the same reason the output is
        // appended before the terminal event: a client woken by an event must
        // not read a session that is behind the run it just heard about.
        if record_input_in_session {
            if let Some(session_id) = session_id {
                self.store.append_session_messages(session_id, base_url, input.clone()).await?;
            }
        }

        let (handle, resume_rx) = RunHandle::new(Arc::clone(&self.store), run);
        handle.spawn_control_listener(control_stream);
        handle.set_created().await?;

        // Record the lineage on the run itself, using the one extension point
        // the specification gives us for agent-defined data.
        if let Some(replaced) = replaces {
            handle
                .emit(Event::generic(serde_json::json!({
                    "replaces": replaced.to_string(),
                    "attempt": attempt,
                })))
                .await?;
        }

        // Shared with the task below rather than handed over. See the drop at
        // the end of that task for why the run, not the agent body, is what
        // this slot's lifetime has to follow.
        let slot = Arc::new(slot);
        let ctx = RunContext::new(
            agent_name,
            run_id,
            input,
            session,
            history,
            base_url.to_string(),
            Arc::clone(&handle),
            resume_rx,
            Arc::clone(&slot),
            self.await_timeout,
        );

        let server = Arc::clone(self);
        let base_url = base_url.to_string();

        // Registered here rather than inside the task. A shutdown landing
        // between the spawn and the task's first poll would otherwise see an
        // idle replica and drain straight past a run that is about to start.
        self.in_flight.enter(run_id);
        tokio::spawn(async move {
            let tracking = Arc::clone(&server);
            execute(server, agent, ctx, handle, session_id, base_url).await;

            // Out of the set first, then the count — the same write-before-
            // signal rule as the terminal event. Releasing the slot is what
            // wakes a drain, and a drain woken by it reads the set in its very
            // next statement; dropping the count first has it report a run
            // that has already finished.
            tracking.in_flight.leave(run_id);

            // Explicit, and last. `ctx` dropped its reference when the agent's
            // body returned, so this is the one that releases the capacity —
            // which is the point: `execute` has by now written the run's output
            // to the session and recorded its outcome, so a drain released here
            // is released against a run that is genuinely finished rather than
            // one still four store writes from it.
            drop(slot);
        });

        Ok((run_id, client_stream))
    }

    /// Start a fresh run to replace one that was abandoned.
    ///
    /// Same agent, same input, same session, new id. The session's history is
    /// *not* extended with the input again — the abandoned run already recorded
    /// it, and duplicating it would corrupt the conversation.
    async fn start_replacement(
        self: &Arc<Self>,
        abandoned: &Run,
        record: &RecoveryRecord,
    ) -> Result<RunId, Error> {
        let agent = self.agents.get(&abandoned.agent_name).cloned().ok_or_else(|| {
            Error::not_found(format!(
                "agent {} is no longer registered on this replica, so run {} cannot be recovered",
                abandoned.agent_name, abandoned.run_id
            ))
        })?;

        let (session, history) = match abandoned.session_id {
            Some(session_id) => {
                let record = self.store.ensure_session(Session::with_id(session_id)).await?;
                (Some(record.session), record.messages)
            }
            None => (None, Vec::new()),
        };

        let base_url = self.base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let (replacement, _) = self
            .launch(
                LaunchSpec {
                    slot: self.in_flight.take(),
                    agent,
                    agent_name: abandoned.agent_name.clone(),
                    input: record.input.clone(),
                    session,
                    history,
                    // A hand-off does not spend an attempt. The ceiling is
                    // there to stop a run that poisons whatever executes it,
                    // and a run whose replica walked away deliberately has
                    // demonstrated nothing of the sort.
                    attempt: if record.handed_off { record.attempt } else { record.attempt + 1 },
                    record_input_in_session: false,
                    replaces: Some(abandoned.run_id),
                },
                &base_url,
            )
            .await?;

        telemetry::recovery_started(abandoned.agent_name.as_str());
        tracing::info!(
            abandoned = %abandoned.run_id,
            %replacement,
            attempt = record.attempt + 1,
            "started a replacement run"
        );
        Ok(replacement)
    }

    /// Determine the session for a run and the history it should see.
    async fn resolve_session(
        &self,
        request: &RunCreateRequest,
    ) -> Result<(Option<Session>, Vec<Message>), Error> {
        let record = match (&request.session, request.session_id) {
            (Some(session), _) => Some(self.store.ensure_session(session.clone()).await?),
            (None, Some(session_id)) => {
                Some(self.store.ensure_session(Session::with_id(session_id)).await?)
            }
            (None, None) => None,
        };
        Ok(match record {
            Some(record) => (Some(record.session), record.messages),
            None => (None, Vec::new()),
        })
    }
}

/// Everything [`AcpServer::launch`] needs to start one run.
struct LaunchSpec {
    /// This replica's permission to run it, held until the run finishes.
    slot: Slot,
    agent: Arc<dyn Agent>,
    agent_name: AgentName,
    input: Vec<Message>,
    session: Option<Session>,
    history: Vec<Message>,
    attempt: u32,
    record_input_in_session: bool,
    replaces: Option<RunId>,
}

/// Reject input the agent has not declared it can consume.
fn check_input_content_types(manifest: &AgentManifest, input: &[Message]) -> Result<(), Error> {
    for message in input {
        for part in &message.parts {
            if !manifest.accepts_input(&part.content_type) {
                return Err(Error::invalid_input(format!(
                    "agent {} does not accept input of type `{}`; supported types: {}",
                    manifest.name,
                    part.content_type,
                    manifest.input_content_types.join(", ")
                )));
            }
        }
    }
    Ok(())
}

/// Keep renewing a run's lease until the task is dropped.
///
/// Runs for as long as the agent does. Aborting it — which happens the moment
/// the run finishes, or the moment the process dies — is what lets the lease
/// lapse and the run be recognised as abandoned.
async fn renew_lease_until_dropped(
    store: Arc<dyn Store>,
    run_id: RunId,
    owner: String,
    ttl: Duration,
) {
    let interval = ttl / LEASE_RENEW_DIVISOR;
    loop {
        tokio::time::sleep(interval).await;
        if let Err(error) = store.renew_lease(run_id, &owner, ttl).await {
            // Losing a renewal is not fatal on its own; several must be missed
            // before the lease lapses. The counter is what turns "one of these
            // scrolled past" into "this is happening more than it used to".
            telemetry::lease_renew_failed();
            tracing::warn!(%run_id, %error, "failed to renew run lease");
        }
    }
}

/// Fail a run whose executing replica is gone, and replace it if it may be
/// replayed.
///
/// A non-terminal run with no live lease has lost its only writer: nothing is
/// left to consume a resume, apply a cancel, or ever write a terminal state.
/// Rather than leave it hanging, mark it failed and publish `run.failed` so
/// waiters on every replica unblock.
///
/// If the agent declared itself [`recoverable`](Agent::recoverable) and the
/// attempt budget is not spent, a **replacement run** is then started here,
/// with the same input and session and a fresh id. The abandoned run keeps its
/// own history and stays failed — the two are linked rather than merged, so no
/// run ever ends up with two sets of output and nothing already streamed to a
/// client is retracted.
///
/// Returns the run as it stands — reaped or not.
pub(crate) async fn reap_if_abandoned(server: &Arc<AcpServer>, run: Run) -> Result<Run, Error> {
    let store = &server.store;
    if run.status.is_terminal() || store.lease_owner(run.run_id).await?.is_some() {
        return Ok(run);
    }

    // A replica on its way out does not adopt someone else's abandoned run.
    // Reaping means either failing the run or starting a replacement *here*,
    // and taking on work during a drain is the one thing a drain exists to
    // stop. Leaving it alone costs nothing: the run stays unowned, and the next
    // replica to read it does the same check with a different answer.
    if !server.is_accepting() {
        tracing::debug!(run_id = %run.run_id, "draining; leaving an abandoned run for another replica");
        return Ok(run);
    }

    let run_id = run.run_id;

    // Failing is idempotent, but starting a replacement is not: two replicas
    // must not both decide to recover the same run. Whoever wins this claim
    // does the work; the others leave the run alone and will see the outcome on
    // their next read.
    let won = store.try_claim_lease(run_id, &server.replica_id, server.lease_ttl).await?;
    telemetry::recovery_claim(won);
    if !won {
        return Ok(run);
    }

    // Created only once a replica has *won* the claim, so exactly one span
    // exists per abandoned run however many replicas noticed it. Everything
    // below — including starting a replacement, which logs on its own — is
    // nested under it.
    //
    // Instrumented rather than entered: the work below awaits, and a sync span
    // guard held across an await attributes whatever else the executor runs to
    // this span.
    let span = tracing::info_span!(
        "acp.reap",
        run_id = %run_id,
        agent = %run.agent_name,
        replica = %server.replica_id,
    );
    reap_claimed(server, run_id).instrument(span).await
}

/// Fail an abandoned run this replica has already claimed the lease on.
async fn reap_claimed(server: &Arc<AcpServer>, run_id: RunId) -> Result<Run, Error> {
    let store = &server.store;

    tracing::warn!(%run_id, "run has no live lease; failing it as abandoned");

    // Winning the claim is not the same as the run still needing to be failed.
    // `run` was read before the lease check, and the executing replica may have
    // reached a terminal state in between — writing its own outcome and
    // releasing the lease, which is what let this claim succeed at all. Failing
    // it now from the stale snapshot would overwrite a completed or cancelled
    // run, breaking the terminal-once rule.
    //
    // The window is microseconds on an in-process store and wide enough to hit
    // every time on one whose round-trips are milliseconds.
    let run = store.require_run(run_id).await?;
    if run.status.is_terminal() {
        store.release_lease(run_id).await?;
        return Ok(run);
    }

    let recovery = store.recovery_record(run_id).await?;
    let replacement = match &recovery {
        Some(record) if record.attempt < server.max_recovery_attempts => {
            match server.start_replacement(&run, record).await {
                Ok(replacement) => Some(replacement),
                Err(error) => {
                    // A failed replacement must not stop the original being
                    // failed, or the client is back to waiting forever.
                    tracing::error!(%run_id, %error, "failed to start a replacement run");
                    None
                }
            }
        }
        Some(record) => {
            telemetry::recovery_exhausted(run.agent_name.as_str());
            tracing::warn!(
                %run_id,
                attempt = record.attempt,
                "not replacing the run: the recovery attempt budget is spent"
            );
            None
        }
        None => None,
    };

    let mut reaped = run;
    reaped.status = RunStatus::Failed;
    reaped.finished_at = Some(chrono::Utc::now());
    reaped.await_request = None;
    reaped.error = Some(match replacement {
        Some(replacement) => Error::server_error(
            "the replica executing this run stopped responding, so the run was abandoned. \
             A replacement run was started; see `data.replaced_by`.",
        )
        // `Error::data` is the specification's own slot for structured detail,
        // so the link travels to the client without inventing a field.
        .with_data(serde_json::json!({ "replaced_by": replacement.to_string() })),
        None => Error::server_error(
            "the replica executing this run stopped responding, so the run was abandoned. \
             Its lease expired without renewal; any output it had already produced is preserved.",
        ),
    });

    store.put_run(&reaped).await?;
    let event = Event::RunFailed { run: Box::new(reaped.clone()) };
    let index = store.append_event(run_id, &event).await?;
    store.publish(run_id, Notification::event_at(index, event)).await?;

    telemetry::run_reaped(reaped.agent_name.as_str());

    // The abandoned run is finished and its input is no longer needed.
    store.put_recovery_record(run_id, None).await?;
    store.release_lease(run_id).await?;

    Ok(reaped)
}

/// Drive one run to a terminal state, inside a span identifying the run.
///
/// The span is what makes a fleet's logs readable: an agent's own output is
/// emitted from inside `agent.run`, so without one it interleaves with every
/// other concurrent run and cannot be told apart afterwards. Everything logged
/// beneath this — by the server or by the agent — carries the run's identity
/// for free.
///
/// Deliberately *not* a request span. A run outlives the request that created
/// it, and can be resumed or cancelled through a different request on a
/// different replica, so a per-request span could not cover it. Per-request
/// spans are `tower-http`'s job, layered on the router like any other
/// middleware.
async fn execute(
    server: Arc<AcpServer>,
    agent: Arc<dyn Agent>,
    ctx: RunContext,
    handle: Arc<RunHandle>,
    session_id: Option<SessionId>,
    base_url: String,
) {
    let run_id = handle.run_id();

    let span = tracing::info_span!(
        "acp.run",
        run_id = %run_id,
        agent = %ctx.agent_name(),
        replica = %server.replica_id,
        // Recorded below rather than inline: most runs have no session, and a
        // field that is sometimes the string "None" is worse than one that is
        // sometimes absent.
        session_id = tracing::field::Empty,
    );
    if let Some(session_id) = session_id {
        span.record("session_id", tracing::field::display(session_id));
    }

    run_to_terminal(server, agent, ctx, handle, session_id, base_url, run_id).instrument(span).await
}

async fn run_to_terminal(
    server: Arc<AcpServer>,
    agent: Arc<dyn Agent>,
    ctx: RunContext,
    handle: Arc<RunHandle>,
    session_id: Option<SessionId>,
    base_url: String,
    run_id: RunId,
) {
    // Held for exactly as long as the agent runs. If this process dies the task
    // dies with it, the lease lapses, and another replica can reap the run.
    let renewal = tokio::spawn(renew_lease_until_dropped(
        Arc::clone(&server.store),
        run_id,
        server.replica_id.clone(),
        server.lease_ttl,
    ));

    if let Err(error) = handle.set_in_progress().await {
        tracing::error!(%run_id, %error, "failed to start run");
        renewal.abort();
        return;
    }

    // Counted here rather than at creation: this is the point the run starts
    // consuming this replica, which is what an in-flight gauge is asked about.
    let agent_name = ctx.agent_name().to_string();
    telemetry::run_started(&agent_name);
    let cancel = handle.cancel_token();

    // The agent body runs in a task of its own so that a panic in it kills only
    // that task. Everything below — the terminal write, leaving the in-flight
    // set, releasing the slot, stopping the lease renewal — has to happen for a
    // run to be finished with, and unwinding through this function skipped all
    // of it. The renewal was the worst of them: a dropped `JoinHandle` detaches
    // rather than cancels, so the orphan kept the lease alive and every reaper
    // that read the run saw a healthy writer and left it alone, forever.
    //
    // `catch_unwind` is not the tool here. The agent's future is held across
    // await points and `RunContext` is not `UnwindSafe`, so the panic has to be
    // caught at a task boundary instead. The cost is one extra spawn per run.
    //
    // Instrumented explicitly, because a spawned task does not inherit the
    // span that was current when it was spawned — it gets whatever is current
    // wherever the runtime happens to poll it, which is nothing. Without this
    // the agent's own log lines fall outside `acp.run` and the correlation #16
    // exists for is gone.
    let mut agent_task =
        tokio::spawn(async move { agent.run(ctx).await }.instrument(tracing::Span::current()));

    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            tracing::debug!(%run_id, "run cancelled");
            agent_task.abort();
            // Awaited, not just aborted. Dropping the future in place used to
            // stop the agent synchronously; an abort only takes effect at the
            // task's next visit to the scheduler, and an emit already in flight
            // would otherwise be free to land after the run is recorded
            // cancelled.
            let _ = (&mut agent_task).await;
            Outcome::Cancelled
        }
        result = &mut agent_task => match result {
            Ok(Ok(())) => Outcome::Completed,
            Ok(Err(error)) => {
                tracing::warn!(%run_id, %error, "agent run failed");
                Outcome::Failed(error)
            }
            Err(join_error) if join_error.is_panic() => {
                let payload = join_error.into_panic();
                // Logged in full, reported in outline. The payload is the one
                // place an operator can find out what actually went wrong, and
                // the one place a remote caller has no business reading — panic
                // messages carry paths, values and whatever else was in scope.
                tracing::error!(%run_id, panic = panic_message(&*payload), "agent panicked");
                Outcome::Failed(Error::server_error(
                    "the agent panicked; see the server logs for the cause",
                ))
            }
            // Aborted by something other than the cancel arm above, which
            // nothing does today. Recorded as a cancellation rather than a
            // failure because that is what it is.
            Err(_) => Outcome::Cancelled,
        },
    };

    // Finish the output and get it into the session *before* the run is marked
    // terminal. That transition publishes the event which releases a `sync`
    // caller, and a caller told its run is done must not then read a session
    // history missing that run's output.
    let outcome = match record_output(&server, &handle, session_id, &base_url).await {
        Ok(()) => outcome,
        // A history write we could not complete is not a run that completed.
        // Same reasoning as emitting: a storage outage should fail the run
        // rather than leave it looking finished with its output missing.
        Err(error) => {
            tracing::error!(%run_id, %error, "failed to append run output to session");
            Outcome::Failed(error)
        }
    };

    let recorded = match outcome {
        Outcome::Completed => handle.set_completed().await,
        Outcome::Cancelled => handle.set_cancelled().await,
        Outcome::Failed(error) => handle.set_failed(error).await,
    };
    if let Err(error) = recorded {
        tracing::error!(%run_id, %error, "failed to record run outcome");
    }

    // Read back rather than inferred from `outcome`: a terminal transition is
    // applied exactly once, so the snapshot is the authority on how the run
    // actually ended and how long it took.
    let final_run = handle.snapshot();
    let elapsed = final_run
        .finished_at
        .map(|finished| finished.signed_duration_since(final_run.created_at))
        .and_then(|delta| delta.to_std().ok());
    telemetry::run_finished(&agent_name, final_run.status, elapsed);

    // The run is finished, so stop claiming it. Releasing is a courtesy — the
    // lease would expire anyway — but it stops a finished run looking owned.
    renewal.abort();
    if let Err(error) = server.store.release_lease(run_id).await {
        // Debug, not warn: the comment above is the reason. The lease expires
        // on its own, and the run is already terminal, so nothing downstream
        // depends on this succeeding — a reaper that finds the stale lease will
        // see a terminal run and leave it alone. Warning here would put a line
        // an operator cannot act on next to ones they must.
        tracing::debug!(%run_id, %error, "failed to release run lease");
    }
    // A finished run will never be replayed, so stop holding its input.
    if let Err(error) = server.store.put_recovery_record(run_id, None).await {
        tracing::warn!(%run_id, %error, "failed to clear the recovery record");
    }
}

/// What a panic said, as far as it can be recovered.
///
/// `panic!` with a literal gives a `&'static str` and one with a format string
/// gives a `String`; anything else came from `panic_any` and there is nothing
/// useful to print. Returning the fallback rather than a `Result` because a
/// panic whose payload cannot be read is still a panic, and the caller has
/// nothing different to do about it.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else {
        "a payload that is not a string"
    }
}

/// How a run ended, before that ending is written down.
///
/// The terminal write is deferred so the session history can be brought up to
/// date first, which means the outcome has to be carried rather than applied
/// where it is decided.
enum Outcome {
    Completed,
    Cancelled,
    Failed(Error),
}

/// Close off the run's output and append it to the session, if it has one.
///
/// Always finalises the output, session or not: an agent that returned
/// mid-message has a message to flush either way.
async fn record_output(
    server: &Arc<AcpServer>,
    handle: &Arc<RunHandle>,
    session_id: Option<SessionId>,
    base_url: &str,
) -> Result<(), Error> {
    let output = handle.finalize_output().await?;

    let Some(session_id) = session_id else { return Ok(()) };
    if output.is_empty() {
        return Ok(());
    }
    server.store.append_session_messages(session_id, base_url, output).await?;
    Ok(())
}

/// Builder for [`AcpServer`].
#[derive(Default)]
pub struct AcpServerBuilder {
    agents: Vec<Arc<dyn Agent>>,
    store: Option<Arc<dyn Store>>,
    base_url: Option<String>,
    max_runs: Option<usize>,
    max_sessions: Option<usize>,
    replica_id: Option<String>,
    lease_ttl: Option<Duration>,
    sync_timeout: Option<Option<Duration>>,
    max_recovery_attempts: Option<u32>,
    max_concurrent_runs: Option<usize>,
    await_timeout: Option<Option<Duration>>,
}

impl std::fmt::Debug for AcpServerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpServerBuilder")
            .field("agents", &self.agents.len())
            .field("store", &self.store)
            .field("base_url", &self.base_url)
            .field("max_runs", &self.max_runs)
            .field("replica_id", &self.replica_id)
            .field("lease_ttl", &self.lease_ttl)
            .finish()
    }
}

impl AcpServerBuilder {
    /// Register an agent.
    pub fn agent(mut self, agent: impl Agent) -> Self {
        self.agents.push(Arc::new(agent));
        self
    }

    /// Register an already-shared agent.
    pub fn shared_agent(mut self, agent: Arc<dyn Agent>) -> Self {
        self.agents.push(agent);
        self
    }

    /// Register several agents at once.
    pub fn agents(mut self, agents: impl IntoIterator<Item = Arc<dyn Agent>>) -> Self {
        self.agents.extend(agents);
        self
    }

    /// Use a specific storage backend.
    ///
    /// Give every replica the same shared store to run several behind a load
    /// balancer. Defaults to an [`InMemoryStore`], which confines a run to the
    /// process that started it.
    ///
    /// Takes precedence over [`max_runs`](AcpServerBuilder::max_runs), which
    /// only configures the default store.
    pub fn store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Set the public base URL used to build session history links.
    ///
    /// When unset, the URL is derived from each request's `Host` header, which
    /// is usually right for direct deployments but not behind a proxy that
    /// rewrites paths — nor across replicas, where links must point at the load
    /// balancer rather than at whichever replica happened to serve the request.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// How long a run may sit `awaiting` a client answer before it is failed.
    ///
    /// Defaults to [`DEFAULT_AWAIT_TIMEOUT`]. A parked conversation costs a
    /// task, a run entry and a lease renewal every few seconds for as long as
    /// it lasts, and nothing else reclaims it — the run is non-terminal with a
    /// live lease, which is indistinguishable from one that is working.
    ///
    /// Distinct from [`sync_timeout`](AcpServerBuilder::sync_timeout), which
    /// bounds how long a *request* waits rather than how long a *run* may be
    /// parked. A `sync` call against a run that parks returns after
    /// `sync_timeout`; the run stays `awaiting` until this elapses.
    pub fn await_timeout(mut self, await_timeout: Duration) -> Self {
        self.await_timeout = Some(Some(await_timeout));
        self
    }

    /// Let a run stay `awaiting` for as long as it likes.
    ///
    /// Right when conversations are genuinely open-ended and the clients are
    /// trusted. On a public address it means anyone who can submit a run can
    /// leave one parked forever, and nothing will reclaim it.
    pub fn without_await_timeout(mut self) -> Self {
        self.await_timeout = Some(None);
        self
    }

    /// Cap how many runs may execute on this replica at once.
    ///
    /// Unset by default, which is unbounded — every run is a spawned task, and
    /// nothing stops a busy enough server from accumulating them until memory
    /// runs out. Set this and `POST /runs` answers **429 with a `Retry-After`**
    /// over the ceiling instead, which is a load an operator can see and a
    /// client can wait out.
    ///
    /// Counts runs *executing an agent body*. A run parked awaiting a client
    /// answer gives its slot up until the answer arrives: it is waiting on a
    /// human who may never return, and holding capacity for it would let idle
    /// conversations starve work that is ready to run. A resumed run takes its
    /// slot back unchecked, so a burst of answers can briefly exceed the
    /// ceiling — this bounds what the replica *takes on*, and stranding a
    /// conversation mid-sentence to defend a number would be the wrong trade.
    ///
    /// Not a rate limit. Requests per second is a tower middleware concern; this
    /// is how many agent invocations are alive at once, which only the server
    /// can know.
    ///
    /// ```
    /// # use rusty_acp::server::AcpServer;
    /// # fn demo() -> Result<(), Box<dyn std::error::Error>> {
    /// let server = AcpServer::builder().max_concurrent_runs(64).build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn max_concurrent_runs(mut self, max_concurrent_runs: usize) -> Self {
        self.max_concurrent_runs = Some(max_concurrent_runs);
        self
    }

    /// Cap how many sessions the default [`InMemoryStore`] retains. Defaults
    /// to [`DEFAULT_MAX_SESSIONS`].
    ///
    /// Past the cap the least recently used session is dropped, along with its
    /// state document. An evicted session is indistinguishable from one that
    /// never existed, so an agent's conversation silently starts over — the
    /// eviction is logged at `warn` for exactly that reason.
    ///
    /// Ignored when a store is supplied with [`store`](AcpServerBuilder::store).
    pub fn max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = Some(max_sessions);
        self
    }

    /// Cap how many runs the default [`InMemoryStore`] retains. Defaults to
    /// [`DEFAULT_MAX_RUNS`]. Active runs are never evicted.
    ///
    /// Ignored when a store is supplied with [`store`](AcpServerBuilder::store).
    pub fn max_runs(mut self, max_runs: usize) -> Self {
        self.max_runs = Some(max_runs);
        self
    }

    /// Name this replica. Defaults to a fresh identifier per process.
    ///
    /// Unlike the other settings here, this one is *meant* to differ between
    /// replicas — it is what identifies the holder of a run's lease.
    ///
    /// The value is the owner recorded on run leases, so a stable, meaningful
    /// name (a pod name, a hostname) makes it obvious in logs which replica was
    /// executing an abandoned run.
    pub fn replica_id(mut self, replica_id: impl Into<String>) -> Self {
        self.replica_id = Some(replica_id.into());
        self
    }

    /// How long a run's lease survives without renewal. Defaults to
    /// [`DEFAULT_LEASE_TTL`].
    ///
    /// This is the window between a replica dying and its runs being failed.
    /// Shorter reacts faster; too short risks mistaking a stalled process for a
    /// dead one and failing a run that was about to finish.
    pub fn lease_ttl(mut self, lease_ttl: Duration) -> Self {
        self.lease_ttl = Some(lease_ttl);
        self
    }

    /// Cap how many times a run may be started before recovery gives up.
    /// Defaults to [`DEFAULT_MAX_RECOVERY_ATTEMPTS`].
    ///
    /// Counts attempts, not retries, so `1` turns recovery off while leaving
    /// abandoned runs still promptly failed.
    ///
    /// Set this to the same value on every replica. The replica that *notices*
    /// an abandoned run is the one that decides whether to replace it, so a
    /// fleet with mismatched budgets behaves like whichever replica got there
    /// first.
    pub fn max_recovery_attempts(mut self, attempts: u32) -> Self {
        self.max_recovery_attempts = Some(attempts.max(1));
        self
    }

    /// How long a `sync` request waits before returning the run as it stands.
    /// Defaults to [`DEFAULT_SYNC_TIMEOUT`].
    ///
    /// The response is still a valid [`Run`]; it just may not be settled, so
    /// callers should check `status` rather than assume it is terminal.
    pub fn sync_timeout(mut self, sync_timeout: Duration) -> Self {
        self.sync_timeout = Some(Some(sync_timeout));
        self
    }

    /// Let `sync` requests wait indefinitely.
    ///
    /// Only sound when something else bounds the request — and note that a
    /// caller whose executing replica dies still unblocks, because the run gets
    /// failed once its lease lapses.
    pub fn without_sync_timeout(mut self) -> Self {
        self.sync_timeout = Some(None);
        self
    }

    /// Validate the configuration and build the server.
    ///
    /// Fails if no agent was registered, if two agents share a name, or if a
    /// manifest violates the specification.
    pub fn build(self) -> Result<AcpServer, Error> {
        // Registering descriptions is idempotent, so doing it per server rather
        // than once globally costs nothing and needs no initialisation call an
        // operator could forget.
        telemetry::describe();

        if self.agents.is_empty() {
            return Err(Error::invalid_input("an ACP server must register at least one agent"));
        }

        let mut agents = HashMap::with_capacity(self.agents.len());
        let mut order = Vec::with_capacity(self.agents.len());
        for agent in self.agents {
            let manifest = agent.manifest();
            manifest.validate()?;
            let name = manifest.name.clone();
            if agents.insert(name.clone(), agent).is_some() {
                return Err(Error::invalid_input(format!("agent {name} is registered twice")));
            }
            order.push(name);
        }

        let store = self.store.unwrap_or_else(|| {
            Arc::new(InMemoryStore::with_limits(
                self.max_runs.unwrap_or(DEFAULT_MAX_RUNS),
                self.max_sessions.unwrap_or(DEFAULT_MAX_SESSIONS),
            ))
        });

        Ok(AcpServer {
            agents,
            order,
            store,
            base_url: self.base_url.map(|url| url.trim_end_matches('/').to_string()),
            replica_id: self
                .replica_id
                .unwrap_or_else(|| format!("replica-{}", uuid::Uuid::new_v4())),
            lease_ttl: self.lease_ttl.unwrap_or(DEFAULT_LEASE_TTL),
            sync_timeout: self.sync_timeout.unwrap_or(Some(DEFAULT_SYNC_TIMEOUT)),
            max_recovery_attempts: self
                .max_recovery_attempts
                .unwrap_or(DEFAULT_MAX_RECOVERY_ATTEMPTS),
            await_timeout: self.await_timeout.unwrap_or(Some(DEFAULT_AWAIT_TIMEOUT)),
            in_flight: Arc::new(InFlight::new(self.max_concurrent_runs)),
            readiness: Mutex::new(None),
        })
    }
}
