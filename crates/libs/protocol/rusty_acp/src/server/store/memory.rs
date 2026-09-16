//! The default in-process [`Store`].

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, RwLock,
    },
    time::{Duration, Instant},
};

use futures_util::stream;
use tokio::sync::broadcast;

use crate::{
    server::store::{
        message_url, state_url, Notification, NotificationStream, RecoveryRecord, SessionRecord,
        Store, StoreResult, DEFAULT_MAX_RUNS, DEFAULT_MAX_RUN_EVENT_BYTES, DEFAULT_MAX_SESSIONS,
    },
    types::{Error, Event, Message, Run, RunId, Session, SessionId},
};

/// Capacity of each run's notification channel.
const CHANNEL_CAPACITY: usize = 256;

#[derive(Debug)]
struct RunEntry {
    /// `None` while only a channel exists — a subscriber can arrive before the
    /// run itself is written.
    run: Option<Run>,
    /// The retained tail of the log. A deque because the bound drops from the
    /// front, and doing that to a `Vec` is a memmove per dropped event.
    events: VecDeque<Event>,
    /// The index the next append will be given.
    ///
    /// Tracked rather than derived from `events.len()`, which is what it used
    /// to be: once the front can be dropped the length stops being the count,
    /// and an index that silently restarts would hand two events the same
    /// `Last-Event-ID`.
    next_index: u64,
    /// The summed [`Event::approximate_size`] of the retained events.
    bytes: usize,
    notifications: broadcast::Sender<Notification>,
}

impl RunEntry {
    fn new(notifications: broadcast::Sender<Notification>, run: Option<Run>) -> Self {
        Self { run, events: VecDeque::new(), next_index: 0, bytes: 0, notifications }
    }

    /// The index of the earliest event still held.
    ///
    /// Everything below this was dropped by the size bound and is gone; a
    /// client asking to resume from before it has to be told rather than handed
    /// a log with a hole in it.
    fn first_index(&self) -> u64 {
        self.next_index - self.events.len() as u64
    }
}

/// Keeps runs and sessions in process memory.
///
/// This is the default backend and needs no configuration. It is the right
/// choice for a single-process agent host; for several replicas behind a load
/// balancer, use a shared backend such as
#[cfg_attr(feature = "redis-store", doc = "[`RedisStore`](super::RedisStore).")]
#[cfg_attr(not(feature = "redis-store"), doc = "`RedisStore` (feature `redis-store`).")]
///
/// Runs are retained up to `max_runs`; past that, the oldest **terminal** runs
/// are evicted first. Active runs are never evicted.
///
/// Sessions are retained up to `max_sessions`, evicting the least recently used
/// — read or appended to — along with its state document. Evicting by age
/// instead would drop the long conversation that is still going in favour of
/// the one nobody has opened since.
///
/// An evicted session is indistinguishable from one that never existed, which
/// means an agent's conversation silently starts over. That is what
/// [`RedisStore`](super::RedisStore)'s TTL already does, so the behaviour is at
/// least consistent across backends; the eviction is logged at `warn` so an
/// operator whose limit is too low learns it from a log rather than from an
/// agent behaving oddly. A tombstone would say more, but tombstones are
/// unbounded unless they are also bounded, which is the same problem one layer
/// down.
///
/// A session in active use is by definition recently touched, so a run's own
/// session is near the back of the queue while it runs. It is not *pinned*
/// though: a long run with more than `max_sessions` fresh sessions started
/// during it can still lose its history. Raise the limit if sessions churn
/// faster than runs complete.
#[derive(Debug)]
pub struct InMemoryStore {
    runs: RwLock<HashMap<RunId, RunEntry>>,
    order: Mutex<VecDeque<RunId>>,
    sessions: RwLock<HashMap<SessionId, SessionEntry>>,
    /// Counts every session touch, so the least recently used is the one with
    /// the smallest stamp. A counter rather than a clock: it cannot go
    /// backwards, and eviction only needs the order.
    session_clock: AtomicU64,
    /// State documents, held apart from the records so `SessionRecord` keeps
    /// carrying a *link* to state rather than the state itself.
    session_states: RwLock<HashMap<SessionId, serde_json::Value>>,
    /// Ownership leases: run to (owner, expiry).
    leases: RwLock<HashMap<RunId, (String, Instant)>>,
    /// Recovery records, present only for runs whose agent opted in.
    recovery: RwLock<HashMap<RunId, RecoveryRecord>>,
    max_runs: usize,
    max_sessions: usize,
    max_run_event_bytes: usize,
}

/// A session, and when it was last used.
#[derive(Debug)]
struct SessionEntry {
    record: SessionRecord,
    /// An atomic so a *read* can mark the session used without taking the
    /// map's write lock. Reads count as use — otherwise the session an agent
    /// keeps reading its history from looks idle and is evicted under it.
    touched: AtomicU64,
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_RUNS)
    }
}

impl InMemoryStore {
    /// A store retaining at most `max_runs` runs, and
    /// [`DEFAULT_MAX_SESSIONS`] sessions.
    pub fn new(max_runs: usize) -> Self {
        Self::with_limits(max_runs, DEFAULT_MAX_SESSIONS)
    }

    /// A store with both bounds set.
    pub fn with_limits(max_runs: usize, max_sessions: usize) -> Self {
        Self {
            runs: RwLock::new(HashMap::new()),
            order: Mutex::new(VecDeque::new()),
            sessions: RwLock::new(HashMap::new()),
            session_clock: AtomicU64::new(0),
            session_states: RwLock::new(HashMap::new()),
            leases: RwLock::new(HashMap::new()),
            recovery: RwLock::new(HashMap::new()),
            max_runs: max_runs.max(1),
            max_sessions: max_sessions.max(1),
            max_run_event_bytes: DEFAULT_MAX_RUN_EVENT_BYTES,
        }
    }

    /// Bound how much of one run's event log is kept.
    ///
    /// Builder-style rather than a third argument to
    /// [`with_limits`](InMemoryStore::with_limits): three bare `usize`s in a
    /// row is an invitation to pass them in the wrong order, and the compiler
    /// would not notice.
    pub fn with_max_run_event_bytes(mut self, max_run_event_bytes: usize) -> Self {
        self.max_run_event_bytes = max_run_event_bytes.max(1);
        self
    }

    /// The size bound on any one run's event log.
    pub fn max_run_event_bytes(&self) -> usize {
        self.max_run_event_bytes
    }

    /// How many runs are currently retained.
    pub fn run_count(&self) -> usize {
        self.runs.read().expect("run map poisoned").len()
    }

    /// How many sessions are currently retained.
    pub fn session_count(&self) -> usize {
        self.sessions.read().expect("session map poisoned").len()
    }

    /// Mark a session as just used.
    fn touch(&self, entry: &SessionEntry) {
        entry.touched.store(self.session_clock.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
    }

    /// Drop least-recently-used sessions until the map is within its bound,
    /// returning what was dropped.
    ///
    /// The caller releases the session lock before clearing the matching state
    /// documents. Taking the state lock here would invert the order
    /// `put_session_state` uses, and an inverted lock order is a deadlock
    /// waiting for the right interleaving.
    fn evict_sessions(&self, sessions: &mut HashMap<SessionId, SessionEntry>) -> Vec<SessionId> {
        let mut evicted = Vec::new();
        while sessions.len() > self.max_sessions {
            let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, entry)| entry.touched.load(Ordering::Relaxed))
                .map(|(id, _)| *id)
            else {
                break;
            };
            sessions.remove(&oldest);
            evicted.push(oldest);
        }
        evicted
    }

    /// Forget the state documents of sessions that have been evicted.
    ///
    /// Not optional: state is usually the larger half of a session, so a bound
    /// that dropped only the record would leave most of the memory behind and
    /// report itself bounded.
    fn forget_session_states(&self, evicted: &[SessionId]) {
        if evicted.is_empty() {
            return;
        }
        let mut states = self.session_states.write().expect("session state map poisoned");
        for session_id in evicted {
            states.remove(session_id);
            // Warn, because the consequence is silent: the next request for
            // this session gets a fresh one, and an agent's conversation starts
            // over with nothing to say it did.
            tracing::warn!(
                %session_id,
                max_sessions = self.max_sessions,
                "evicted the least recently used session; its history and state are gone"
            );
        }
    }

    /// Get the sender for a run's channel, creating the entry if needed.
    fn channel(&self, run_id: RunId) -> broadcast::Sender<Notification> {
        if let Some(entry) = self.runs.read().expect("run map poisoned").get(&run_id) {
            return entry.notifications.clone();
        }
        // A subscriber can arrive before the run is written; give it a channel
        // to wait on rather than dropping the subscription.
        let mut runs = self.runs.write().expect("run map poisoned");
        match runs.get(&run_id) {
            Some(entry) => entry.notifications.clone(),
            None => {
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                runs.insert(run_id, RunEntry::new(tx.clone(), None));
                self.order.lock().expect("run order poisoned").push_back(run_id);
                tx
            }
        }
    }

    fn evict(&self, runs: &mut HashMap<RunId, RunEntry>) {
        let mut order = self.order.lock().expect("run order poisoned");
        while runs.len() > self.max_runs {
            let Some(position) = order.iter().position(|id| {
                runs.get(id).is_some_and(|entry| {
                    entry.run.as_ref().is_some_and(|run| run.status.is_terminal())
                })
            }) else {
                // Every retained run is still active; keep them all.
                break;
            };
            if let Some(id) = order.remove(position) {
                runs.remove(&id);
            }
        }
    }
}

#[async_trait::async_trait]
impl Store for InMemoryStore {
    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        let mut runs = self.runs.write().expect("run map poisoned");
        match runs.get_mut(&run.run_id) {
            Some(entry) => entry.run = Some(run.clone()),
            None => {
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                runs.insert(run.run_id, RunEntry::new(tx, Some(run.clone())));
                self.order.lock().expect("run order poisoned").push_back(run.run_id);
            }
        }
        self.evict(&mut runs);
        Ok(())
    }

    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        Ok(self
            .runs
            .read()
            .expect("run map poisoned")
            .get(&run_id)
            .and_then(|entry| entry.run.clone()))
    }

    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
        let limit = self.max_run_event_bytes;
        let mut runs = self.runs.write().expect("run map poisoned");
        let Some(entry) = runs.get_mut(&run_id) else {
            return Err(Error::not_found(format!("run {run_id} not found")));
        };

        // The write lock is what makes the index unique: a concurrent append
        // cannot observe the same counter.
        let index = entry.next_index;
        entry.bytes += event.approximate_size();
        entry.events.push_back(event.clone());
        entry.next_index += 1;

        // Trimmed after appending, not before, so the event just emitted is
        // always retained — a log that dropped what it was being given could
        // not serve even a live tail.
        let mut dropped = 0u64;
        while entry.bytes > limit && entry.events.len() > 1 {
            if let Some(oldest) = entry.events.pop_front() {
                entry.bytes = entry.bytes.saturating_sub(oldest.approximate_size());
                dropped += 1;
            }
        }
        if dropped > 0 {
            // Warn, and once per trim rather than once per event, because the
            // consequence is only visible to a client that later tries to
            // resume from what is now gone.
            tracing::warn!(
                %run_id,
                dropped,
                first_index = entry.first_index(),
                limit,
                "dropped the oldest events of a run past the log size bound"
            );
        }
        Ok(index)
    }

    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        Ok(self
            .runs
            .read()
            .expect("run map poisoned")
            .get(&run_id)
            .map(|entry| entry.events.iter().cloned().collect())
            .unwrap_or_default())
    }

    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        let runs = self.runs.read().expect("run map poisoned");
        let Some(entry) = runs.get(&run_id) else {
            return Ok(Vec::new());
        };
        // Relative to the earliest event still held, not to zero: with a
        // trimmed front those stopped being the same number, and skipping by
        // the absolute index would silently return the wrong events.
        let skip = from.saturating_sub(entry.first_index()) as usize;
        Ok(entry.events.iter().skip(skip).cloned().collect())
    }

    async fn earliest_event(&self, run_id: RunId) -> StoreResult<u64> {
        Ok(self
            .runs
            .read()
            .expect("run map poisoned")
            .get(&run_id)
            .map_or(0, RunEntry::first_index))
    }

    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        // No subscribers is not an error: the event log is the durable record.
        let _ = self.channel(run_id).send(notification);
        Ok(())
    }

    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        let receiver = self.channel(run_id).subscribe();
        Ok(Box::pin(stream::unfold(receiver, |mut receiver| async move {
            match receiver.recv().await {
                Ok(notification) => Some((notification, receiver)),
                Err(broadcast::error::RecvError::Closed) => None,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    // Surface the gap rather than silently dropping it; the
                    // full log is still available from `events`.
                    let event = Event::Error {
                        error: Error::server_error(format!(
                            "notification stream lagged; {skipped} messages were dropped. \
                             Fetch GET /runs/{{run_id}}/events for the full log."
                        )),
                    };
                    Some((Notification::unlogged_event(event), receiver))
                }
            }
        })))
    }

    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        let sessions = self.sessions.read().expect("session map poisoned");
        Ok(sessions.get(&session_id).map(|entry| {
            self.touch(entry);
            entry.record.clone()
        }))
    }

    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        let (record, evicted) = {
            let mut sessions = self.sessions.write().expect("session map poisoned");
            let entry = sessions.entry(session.id).or_insert_with(|| SessionEntry {
                record: SessionRecord::new(session),
                touched: AtomicU64::new(0),
            });
            self.touch(entry);
            let record = entry.record.clone();
            (record, self.evict_sessions(&mut sessions))
        };
        self.forget_session_states(&evicted);
        Ok(record)
    }

    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        let evicted = {
            // Holding the write lock across the whole append keeps index and
            // URL in step even when two runs append concurrently.
            let mut sessions = self.sessions.write().expect("session map poisoned");
            let entry = sessions.entry(session_id).or_insert_with(|| SessionEntry {
                record: SessionRecord::new(Session::with_id(session_id)),
                touched: AtomicU64::new(0),
            });
            self.touch(entry);
            for message in messages {
                let index = entry.record.messages.len();
                entry.record.messages.push(message);
                entry.record.session.history.push(message_url(base_url, session_id, index));
            }
            self.evict_sessions(&mut sessions)
        };
        self.forget_session_states(&evicted);
        Ok(())
    }

    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
        self.leases
            .write()
            .expect("lease map poisoned")
            .insert(run_id, (owner.to_string(), Instant::now() + ttl));
        Ok(())
    }

    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        Ok(self.leases.read().expect("lease map poisoned").get(&run_id).and_then(
            |(owner, expires_at)| {
                // Expiry is checked on read rather than swept: an expired lease
                // is indistinguishable from an absent one, which is all callers
                // need to know.
                (*expires_at > Instant::now()).then(|| owner.clone())
            },
        ))
    }

    async fn try_claim_lease(
        &self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> StoreResult<bool> {
        // Held across the check and the insert, so two claimants cannot both
        // see the slot empty.
        let mut leases = self.leases.write().expect("lease map poisoned");
        let taken = leases.get(&run_id).is_some_and(|(_, expires_at)| *expires_at > Instant::now());
        if taken {
            return Ok(false);
        }
        leases.insert(run_id, (owner.to_string(), Instant::now() + ttl));
        Ok(true)
    }

    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        let mut recovery = self.recovery.write().expect("recovery map poisoned");
        match record {
            Some(record) => recovery.insert(run_id, record.clone()),
            None => recovery.remove(&run_id),
        };
        Ok(())
    }

    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        Ok(self.recovery.read().expect("recovery map poisoned").get(&run_id).cloned())
    }

    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        self.leases.write().expect("lease map poisoned").remove(&run_id);
        Ok(())
    }

    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        // Reading state is using the session, so it counts against eviction the
        // same way reading history does.
        if let Some(entry) = self.sessions.read().expect("session map poisoned").get(&session_id) {
            self.touch(entry);
        }
        Ok(self
            .session_states
            .read()
            .expect("session state map poisoned")
            .get(&session_id)
            .cloned())
    }

    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        self.session_states.write().expect("session state map poisoned").insert(session_id, state);
        let evicted = {
            // Point the session at the document rather than inlining it.
            let mut sessions = self.sessions.write().expect("session map poisoned");
            let entry = sessions.entry(session_id).or_insert_with(|| SessionEntry {
                record: SessionRecord::new(Session::with_id(session_id)),
                touched: AtomicU64::new(0),
            });
            self.touch(entry);
            entry.record.session.state = Some(state_url(base_url, session_id));
            self.evict_sessions(&mut sessions)
        };
        self.forget_session_states(&evicted);
        Ok(())
    }
}
