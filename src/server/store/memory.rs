//! The default in-process [`Store`].

use std::{
    collections::{HashMap, VecDeque},
    sync::{Mutex, RwLock},
    time::{Duration, Instant},
};

use futures_util::stream;
use tokio::sync::broadcast;

use crate::{
    server::store::{
        message_url, state_url, Notification, NotificationStream, RecoveryRecord, SessionRecord,
        Store, StoreResult, DEFAULT_MAX_RUNS,
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
    events: Vec<Event>,
    notifications: broadcast::Sender<Notification>,
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
#[derive(Debug)]
pub struct InMemoryStore {
    runs: RwLock<HashMap<RunId, RunEntry>>,
    order: Mutex<VecDeque<RunId>>,
    sessions: RwLock<HashMap<SessionId, SessionRecord>>,
    /// State documents, held apart from the records so `SessionRecord` keeps
    /// carrying a *link* to state rather than the state itself.
    session_states: RwLock<HashMap<SessionId, serde_json::Value>>,
    /// Ownership leases: run to (owner, expiry).
    leases: RwLock<HashMap<RunId, (String, Instant)>>,
    /// Recovery records, present only for runs whose agent opted in.
    recovery: RwLock<HashMap<RunId, RecoveryRecord>>,
    max_runs: usize,
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_RUNS)
    }
}

impl InMemoryStore {
    /// A store retaining at most `max_runs` runs.
    pub fn new(max_runs: usize) -> Self {
        Self {
            runs: RwLock::new(HashMap::new()),
            order: Mutex::new(VecDeque::new()),
            sessions: RwLock::new(HashMap::new()),
            session_states: RwLock::new(HashMap::new()),
            leases: RwLock::new(HashMap::new()),
            recovery: RwLock::new(HashMap::new()),
            max_runs: max_runs.max(1),
        }
    }

    /// How many runs are currently retained.
    pub fn run_count(&self) -> usize {
        self.runs.read().expect("run map poisoned").len()
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
                runs.insert(
                    run_id,
                    RunEntry { run: None, events: Vec::new(), notifications: tx.clone() },
                );
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
                runs.insert(
                    run.run_id,
                    RunEntry { run: Some(run.clone()), events: Vec::new(), notifications: tx },
                );
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
        let mut runs = self.runs.write().expect("run map poisoned");
        match runs.get_mut(&run_id) {
            // The write lock is what makes the index unique: a concurrent
            // append cannot observe the same length.
            Some(entry) => {
                entry.events.push(event.clone());
                Ok(entry.events.len() as u64 - 1)
            }
            None => Err(Error::not_found(format!("run {run_id} not found"))),
        }
    }

    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        Ok(self
            .runs
            .read()
            .expect("run map poisoned")
            .get(&run_id)
            .map(|entry| entry.events.clone())
            .unwrap_or_default())
    }

    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        Ok(self
            .runs
            .read()
            .expect("run map poisoned")
            .get(&run_id)
            .map(|entry| entry.events.iter().skip(from as usize).cloned().collect())
            .unwrap_or_default())
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
        Ok(self.sessions.read().expect("session map poisoned").get(&session_id).cloned())
    }

    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        let mut sessions = self.sessions.write().expect("session map poisoned");
        Ok(sessions.entry(session.id).or_insert_with(|| SessionRecord::new(session)).clone())
    }

    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        // Holding the write lock across the whole append keeps index and URL
        // in step even when two runs append concurrently.
        let mut sessions = self.sessions.write().expect("session map poisoned");
        let record = sessions
            .entry(session_id)
            .or_insert_with(|| SessionRecord::new(Session::with_id(session_id)));
        for message in messages {
            let index = record.messages.len();
            record.messages.push(message);
            record.session.history.push(message_url(base_url, session_id, index));
        }
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
        // Point the session at the document rather than inlining it.
        let mut sessions = self.sessions.write().expect("session map poisoned");
        let record = sessions
            .entry(session_id)
            .or_insert_with(|| SessionRecord::new(Session::with_id(session_id)));
        record.session.state = Some(state_url(base_url, session_id));
        Ok(())
    }
}
