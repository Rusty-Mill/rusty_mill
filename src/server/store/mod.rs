//! Pluggable storage backends.
//!
//! A [`Store`] holds everything about a run that must outlive — or reach
//! beyond — the process handling a given request: the run snapshot, its event
//! log, its session, and a per-run publish/subscribe channel.
//!
//! # Why this is a trait
//!
//! ACP's [high-availability guide][ha] calls for centralised storage so several
//! server replicas can sit behind a load balancer without session affinity.
//! Swapping [`InMemoryStore`] for a shared backend such as [`RedisStore`] is
//! all it takes: every endpoint reads and writes through this trait, so any
//! replica can serve any request for any run.
//!
//! # The sole-writer invariant
//!
//! **The replica executing a run is the only writer of that run's state.**
//! Other replicas read snapshots and send control signals ([`Notification::Resume`],
//! [`Notification::Cancel`]) to the executing replica, which applies them and
//! writes the result. Nothing else mutates a run.
//!
//! That is what lets [`Store::put_run`] be a plain overwrite with no
//! compare-and-swap or distributed lock: there is never more than one writer
//! for a given run. Implementors do not need to serialise concurrent writes to
//! the same run, because there are none.
//!
//! The invariant's weak point is that a writer can *die*. A run would then stay
//! non-terminal forever, with nothing left to consume a resume or apply a
//! cancel. [`Store::renew_lease`] closes that: the executing replica keeps a
//! short-lived lease, so a non-terminal run whose lease has lapsed is
//! recognisably abandoned and gets failed rather than hanging.
//!
//! # One channel for two jobs
//!
//! [`Notification`] carries both directions of traffic over a single
//! per-run channel:
//!
//! - [`Notification::Event`] fans **out** — to streaming clients connected to
//!   any replica, and to `sync`-mode callers waiting for the run to settle.
//! - [`Notification::Resume`] and [`Notification::Cancel`] route **in** — to
//!   whichever replica is executing the run.
//!
//! A backend therefore needs exactly one pub/sub primitive: a Redis channel, a
//! Postgres `LISTEN`/`NOTIFY` channel, or an in-process broadcast.
//!
//! [ha]: https://agentcommunicationprotocol.dev/how-to/high-availability

mod memory;

#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
mod redis;

use std::{pin::Pin, time::Duration};

use futures_util::Stream;
use serde::{Deserialize, Serialize};

use crate::types::{AwaitResume, Error, Event, Message, Run, RunId, Session, SessionId};

pub use memory::InMemoryStore;

#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
pub use redis::{RedisStore, RedisStoreConfig};

/// Default number of runs an [`InMemoryStore`] retains before evicting
/// terminal ones.
pub const DEFAULT_MAX_RUNS: usize = 1024;

/// Result of a store operation.
///
/// Backends report failures as [`Error`] so they surface to clients as ordinary
/// ACP errors — a storage outage becomes a `server_error`, not a panic.
pub type StoreResult<T> = Result<T, Error>;

/// A stream of [`Notification`]s for one run.
pub type NotificationStream = Pin<Box<dyn Stream<Item = Notification> + Send>>;

/// A message published on a run's channel.
///
/// See the [module docs](self#one-channel-for-two-jobs) for why events and
/// control signals share one channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notification {
    /// An event was emitted by the run. Fans out to every subscriber.
    Event(Event),
    /// A client supplied the payload an awaiting run was waiting for.
    /// Consumed by the replica executing the run.
    Resume(AwaitResume),
    /// A client requested cancellation. Consumed by the replica executing the
    /// run, which decides when the run actually stops.
    Cancel,
}

impl Notification {
    /// The event, when this is an [`Notification::Event`].
    pub fn event(&self) -> Option<&Event> {
        match self {
            Notification::Event(event) => Some(event),
            _ => None,
        }
    }
}

/// A session together with the messages a store holds for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// The session as returned by `GET /session/{session_id}`.
    pub session: Session,
    /// Messages stored for this session, in order. Each corresponds to one
    /// entry appended to [`Session::history`].
    pub messages: Vec<Message>,
}

impl SessionRecord {
    /// An empty record for a new session.
    pub fn new(session: Session) -> Self {
        Self { session, messages: Vec::new() }
    }
}

/// Storage and messaging backend for runs and sessions.
///
/// Implement this to put runs somewhere other than process memory. See the
/// [module docs](self) for the invariants a backend may rely on.
#[async_trait::async_trait]
pub trait Store: Send + Sync + std::fmt::Debug + 'static {
    /// Write a run snapshot, replacing any previous one.
    ///
    /// A plain overwrite is safe: see the
    /// [sole-writer invariant](self#the-sole-writer-invariant).
    async fn put_run(&self, run: &Run) -> StoreResult<()>;

    /// Read a run snapshot, or `None` if the store has no such run.
    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>>;

    /// Append an event to a run's log.
    ///
    /// The log backs `GET /runs/{run_id}/events` and must preserve order.
    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<()>;

    /// Read a run's full event log, in emission order.
    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>>;

    /// Publish a notification on a run's channel.
    ///
    /// Delivery is best-effort and fan-out: every live subscriber on every
    /// replica should receive it. Publishing to a run with no subscribers is
    /// not an error.
    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()>;

    /// Subscribe to a run's channel.
    ///
    /// Must not return until the subscription is live, so a caller can
    /// subscribe and then act without missing what its own action triggers.
    /// Because there is no replay, callers should re-read the run after
    /// subscribing to catch anything that happened first — see
    /// [`Store::get_run`].
    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream>;

    /// Read a session record, or `None` if the store has no such session.
    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>>;

    /// Create a session if it does not exist, returning the current record.
    ///
    /// Seeds a new session from `session`, preserving history hosted by other
    /// servers. An existing session is returned untouched.
    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord>;

    /// Append messages to a session, extending its history with URLs that
    /// resolve against `base_url`.
    ///
    /// Must be atomic with respect to other appends to the same session:
    /// two replicas appending concurrently must not interleave or lose
    /// messages, and each message's history URL must match its stored index.
    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()>;

    /// Read the state document an agent stored for a session.
    ///
    /// Returns `None` when the session has no state, or does not exist.
    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>>;

    /// Replace a session's state document.
    ///
    /// Must also point [`Session::state`] at the document, using
    /// [`state_url`] against `base_url`. ACP models state as a *link* rather
    /// than inline content, so `GET /session/{id}` stays small however large
    /// the state grows.
    ///
    /// State is scoped to the session and outlives any single run.
    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()>;

    /// Claim or extend this replica's ownership of a run.
    ///
    /// The lease is what makes the [sole-writer invariant](self#the-sole-writer-invariant)
    /// survivable: while a replica executes a run it keeps renewing the lease,
    /// so a **non-terminal run with no live lease has lost its writer** and can
    /// be recognised as abandoned rather than hanging forever.
    ///
    /// Implementations must expire the lease `ttl` after the last renewal,
    /// without needing anyone to come back and delete it — the whole point is
    /// that it outlives a replica that stopped being able to do anything.
    ///
    /// Renewing is unconditional: the executing replica is the only writer, so
    /// there is no other claimant to lose a race with.
    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()>;

    /// The replica currently holding a run's lease, or `None` if the lease has
    /// expired or was never taken.
    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>>;

    /// Drop a run's lease, once it has reached a terminal state.
    ///
    /// Releasing is an optimisation, not a requirement: an unreleased lease
    /// simply expires. It exists so a finished run stops looking owned straight
    /// away.
    async fn release_lease(&self, run_id: RunId) -> StoreResult<()>;

    /// Read a run, or produce a `not_found` error.
    async fn require_run(&self, run_id: RunId) -> StoreResult<Run> {
        self.get_run(run_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("run {run_id} not found")))
    }

    /// Read a session, or produce a `not_found` error.
    async fn require_session(&self, session_id: SessionId) -> StoreResult<SessionRecord> {
        self.get_session(session_id)
            .await?
            .ok_or_else(|| Error::not_found(format!("session {session_id} not found")))
    }
}

/// Build the resource URL for a stored session message.
///
/// These are the URLs that populate [`Session::history`], which ACP defines as
/// links to messages rather than inline content.
pub fn message_url(base_url: &str, session_id: SessionId, index: usize) -> String {
    format!("{}/session/{}/messages/{}", base_url.trim_end_matches('/'), session_id, index)
}

/// Build the resource URL for a session's state document.
///
/// This is the URL that populates [`Session::state`]. Like history, ACP models
/// state as a link rather than inline content.
pub fn state_url(base_url: &str, session_id: SessionId) -> String {
    format!("{}/session/{}/state", base_url.trim_end_matches('/'), session_id)
}
