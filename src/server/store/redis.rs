//! A Redis-backed [`Store`], for running several replicas behind a load
//! balancer.

use std::time::Duration;

use futures_util::StreamExt;
use redis::{aio::ConnectionManager, AsyncCommands, Client};

use crate::{
    server::store::{
        message_url, state_url, Notification, NotificationStream, RecoveryRecord, SessionRecord,
        Store, StoreResult,
    },
    types::{Error, Event, Message, Run, RunId, Session, SessionId},
};

/// Default time-to-live applied to run, event and session keys.
pub const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Default prefix for every key and channel this store owns.
pub const DEFAULT_KEY_PREFIX: &str = "acp";

/// How a [`RedisStore`] names and expires its keys.
#[derive(Debug, Clone)]
pub struct RedisStoreConfig {
    /// Prefix for every key and channel. Lets several deployments share one
    /// Redis instance.
    pub key_prefix: String,
    /// How long runs, event logs and sessions live after their last write.
    ///
    /// The ACP high-availability guide calls for automatic expiration; this is
    /// how it is applied. `None` keeps keys forever, which needs external
    /// cleanup.
    pub ttl: Option<Duration>,
    /// How much of one run's event log is kept, in bytes.
    ///
    /// A TTL bounds how *long* a log is kept, not how *much*: a single
    /// streaming run can exhaust an instance well inside its window. Matches
    /// [`InMemoryStore`](super::InMemoryStore)'s
    /// [`max_run_event_bytes`](super::InMemoryStore::max_run_event_bytes) so
    /// the backends agree on what a byte of log is, and measured with
    /// [`Event::approximate_size`] for the same reason.
    pub max_run_event_bytes: usize,
}

impl Default for RedisStoreConfig {
    fn default() -> Self {
        Self {
            key_prefix: DEFAULT_KEY_PREFIX.to_string(),
            ttl: Some(DEFAULT_TTL),
            max_run_event_bytes: super::DEFAULT_MAX_RUN_EVENT_BYTES,
        }
    }
}

/// Session metadata held alongside the message list.
///
/// History URLs are *derived* from message indices at read time rather than
/// stored, which is what lets an append be a single atomic `RPUSH`: the URL for
/// message `i` is a pure function of `i`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SessionMeta {
    id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    /// History entries supplied by the client, hosted elsewhere. These come
    /// before anything this deployment stores.
    #[serde(default)]
    prefix_history: Vec<String>,
    /// Base URL recorded on first append, so reads can rebuild history URLs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
}

/// Keeps runs and sessions in Redis, using pub/sub for notifications.
///
/// Point every replica at the same Redis instance and they share one view of
/// every run: any replica can serve any request, and control signals reach
/// whichever replica is executing the agent. This is the backend the ACP
/// [high-availability guide][ha] names first.
///
/// ```no_run
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// use rusty_acp::server::store::RedisStore;
///
/// let store = RedisStore::connect("redis://127.0.0.1/").await?;
/// # Ok(())
/// # }
/// ```
///
/// [ha]: https://agentcommunicationprotocol.dev/how-to/high-availability
#[derive(Clone)]
pub struct RedisStore {
    client: Client,
    connection: ConnectionManager,
    config: RedisStoreConfig,
}

impl std::fmt::Debug for RedisStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisStore").field("config", &self.config).finish_non_exhaustive()
    }
}

impl RedisStore {
    /// Connect with the default configuration.
    pub async fn connect(url: &str) -> StoreResult<Self> {
        Self::connect_with(url, RedisStoreConfig::default()).await
    }

    /// Connect with an explicit configuration.
    pub async fn connect_with(url: &str, config: RedisStoreConfig) -> StoreResult<Self> {
        let client = Client::open(url).map_err(|err| redis_error("open Redis client", err))?;
        let connection = ConnectionManager::new(client.clone())
            .await
            .map_err(|err| redis_error("connect to Redis", err))?;
        Ok(Self { client, connection, config })
    }

    /// The configuration in use.
    pub fn config(&self) -> &RedisStoreConfig {
        &self.config
    }

    fn run_key(&self, run_id: RunId) -> String {
        format!("{}:run:{run_id}", self.config.key_prefix)
    }

    fn events_key(&self, run_id: RunId) -> String {
        format!("{}:events:{run_id}", self.config.key_prefix)
    }

    /// The index the next append to this run's log will be given.
    ///
    /// A counter of its own rather than the list's length, which is what it
    /// used to be. `RPUSH` returns the new length and that made the index free
    /// — until the front of the list became droppable, at which point the
    /// length stops being the count and two different events would be handed
    /// the same `Last-Event-ID`. In memory that was caught by a test; here it
    /// would have arrived as a resuming client silently skipping or repeating,
    /// with nothing failing.
    fn next_index_key(&self, run_id: RunId) -> String {
        format!("{}:next:{run_id}", self.config.key_prefix)
    }

    /// The summed [`Event::approximate_size`] of this run's retained events.
    ///
    /// Tracked rather than derived, because asking Redis how many bytes a list
    /// holds means reading the list back.
    fn events_bytes_key(&self, run_id: RunId) -> String {
        format!("{}:evbytes:{run_id}", self.config.key_prefix)
    }

    fn channel(&self, run_id: RunId) -> String {
        format!("{}:chan:{run_id}", self.config.key_prefix)
    }

    fn session_meta_key(&self, session_id: SessionId) -> String {
        format!("{}:session:{session_id}:meta", self.config.key_prefix)
    }

    fn session_messages_key(&self, session_id: SessionId) -> String {
        format!("{}:session:{session_id}:messages", self.config.key_prefix)
    }

    fn lease_key(&self, run_id: RunId) -> String {
        format!("{}:lease:{run_id}", self.config.key_prefix)
    }

    fn recovery_key(&self, run_id: RunId) -> String {
        format!("{}:recovery:{run_id}", self.config.key_prefix)
    }

    fn session_state_key(&self, session_id: SessionId) -> String {
        format!("{}:session:{session_id}:state", self.config.key_prefix)
    }

    fn ttl_seconds(&self) -> Option<i64> {
        self.config.ttl.map(|ttl| ttl.as_secs().max(1) as i64)
    }

    /// Refresh a key's expiry, if a TTL is configured.
    async fn touch(&self, key: &str) -> StoreResult<()> {
        if let Some(seconds) = self.ttl_seconds() {
            let mut connection = self.connection.clone();
            let _: () = connection
                .expire(key, seconds)
                .await
                .map_err(|err| redis_error("set key expiry", err))?;
        }
        Ok(())
    }
}

/// A lease TTL in milliseconds, for `PX`.
///
/// Seconds — `SET EX`, which this used until the conformance suite in #69 asked
/// a store to honour a sub-second lease — cannot express what the trait
/// promises. `Duration::as_secs` *truncates*, so a 1500ms lease expired after
/// one, and `.max(1)` then rounded a 500ms lease *up* to a second. Expiring
/// early is the dangerous half: a lease that lapses under a replica still
/// renewing it gets a live run reaped as abandoned.
///
/// `PX` has been in Redis since 2.6 and costs nothing, so there was never much
/// to weigh — the seconds version was simply the first thing written.
fn lease_millis(ttl: Duration) -> u64 {
    // Saturating rather than wrapping: a nonsensically large TTL should pin the
    // lease open, not wrap round to a short one.
    u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX).max(1)
}

fn redis_error(action: &str, err: redis::RedisError) -> Error {
    Error::server_error(format!("failed to {action}: {err}"))
}

fn encode<T: serde::Serialize>(value: &T) -> StoreResult<String> {
    serde_json::to_string(value)
        .map_err(|err| Error::server_error(format!("failed to encode for Redis: {err}")))
}

fn decode<T: serde::de::DeserializeOwned>(raw: &str) -> StoreResult<T> {
    serde_json::from_str(raw)
        .map_err(|err| Error::server_error(format!("failed to decode from Redis: {err}")))
}

#[async_trait::async_trait]
impl Store for RedisStore {
    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        let key = self.run_key(run.run_id);
        let mut connection = self.connection.clone();
        let payload = encode(run)?;
        match self.ttl_seconds() {
            Some(seconds) => {
                let _: () = connection
                    .set_ex(&key, payload, seconds as u64)
                    .await
                    .map_err(|err| redis_error("write run", err))?;
            }
            None => {
                let _: () = connection
                    .set(&key, payload)
                    .await
                    .map_err(|err| redis_error("write run", err))?;
            }
        }
        Ok(())
    }

    async fn check_health(&self) -> StoreResult<()> {
        let mut connection = self.connection.clone();
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .map(|_| ())
            .map_err(|err| redis_error("ping", err))
    }

    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = connection
            .get(self.run_key(run_id))
            .await
            .map_err(|err| redis_error("read run", err))?;
        raw.as_deref().map(decode).transpose()
    }

    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
        let key = self.events_key(run_id);
        let index_key = self.next_index_key(run_id);
        let bytes_key = self.events_bytes_key(run_id);
        let mut connection = self.connection.clone();

        // Several commands rather than one, and deliberately not wrapped in a
        // transaction or a Lua script. The sole-writer invariant is what makes
        // that safe: one replica writes a given run, and it appends that run's
        // events in order, so nothing can interleave between these. Two runs
        // touch different keys entirely.
        let next: u64 = connection
            .incr(&index_key, 1)
            .await
            .map_err(|err| redis_error("reserve event index", err))?;
        let index = next - 1;

        let size = event.approximate_size();
        let _: () = connection
            .rpush(&key, encode(event)?)
            .await
            .map_err(|err| redis_error("append event", err))?;
        let mut held: i64 = connection
            .incr(&bytes_key, size as i64)
            .await
            .map_err(|err| redis_error("account for event size", err))?;

        // Trimmed after appending, so the event just emitted is always kept —
        // a log that dropped what it was being given could not serve even a
        // live tail.
        let limit = self.config.max_run_event_bytes as i64;
        let mut dropped = 0u64;
        while held > limit {
            let length: u64 =
                connection.llen(&key).await.map_err(|err| redis_error("measure event log", err))?;
            if length <= 1 {
                break;
            }
            let Some(oldest): Option<String> =
                connection.lpop(&key, None).await.map_err(|err| redis_error("trim log", err))?
            else {
                break;
            };
            let freed = decode::<Event>(&oldest).map(|event| event.approximate_size()).unwrap_or(0);
            held = connection
                .decr(&bytes_key, freed as i64)
                .await
                .map_err(|err| redis_error("account for a trimmed event", err))?;
            dropped += 1;
        }
        if dropped > 0 {
            tracing::warn!(
                %run_id,
                dropped,
                limit,
                "dropped the oldest events of a run past the log size bound"
            );
        }

        self.touch(&key).await?;
        self.touch(&index_key).await?;
        self.touch(&bytes_key).await?;
        Ok(index)
    }

    async fn earliest_event(&self, run_id: RunId) -> StoreResult<u64> {
        let mut connection = self.connection.clone();
        let next: Option<u64> = connection
            .get(self.next_index_key(run_id))
            .await
            .map_err(|err| redis_error("read the next event index", err))?;
        let length: u64 = connection
            .llen(self.events_key(run_id))
            .await
            .map_err(|err| redis_error("measure event log", err))?;
        // What was appended, less what is still held. Derived rather than
        // stored, so the two can never disagree.
        Ok(next.unwrap_or(0).saturating_sub(length))
    }

    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        let mut connection = self.connection.clone();
        let raw: Vec<String> = connection
            .lrange(self.events_key(run_id), 0, -1)
            .await
            .map_err(|err| redis_error("read events", err))?;
        raw.iter().map(|entry| decode(entry)).collect()
    }

    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        // Relative to the earliest event still held, not to zero. With a
        // trimmed front those stopped being the same number, and seeking by
        // the absolute index would return real events from the wrong position
        // — worse than returning none.
        let earliest = self.earliest_event(run_id).await?;
        let offset = from.saturating_sub(earliest);

        let mut connection = self.connection.clone();
        // LRANGE seeks, so a reconnection costs the tail rather than the whole
        // run so far.
        let raw: Vec<String> = connection
            .lrange(self.events_key(run_id), offset as isize, -1)
            .await
            .map_err(|err| redis_error("read events", err))?;
        raw.iter().map(|entry| decode(entry)).collect()
    }

    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        let mut connection = self.connection.clone();
        // No subscribers is not an error: the event log is the durable record.
        let _: () = connection
            .publish(self.channel(run_id), encode(&notification)?)
            .await
            .map_err(|err| redis_error("publish notification", err))?;
        Ok(())
    }

    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|err| redis_error("open Redis pub/sub connection", err))?;
        // Returns only once Redis has acknowledged the subscription, so a
        // caller can subscribe and then act without missing its own effects.
        pubsub
            .subscribe(self.channel(run_id))
            .await
            .map_err(|err| redis_error("subscribe to run channel", err))?;

        let stream = pubsub.into_on_message().filter_map(|message| async move {
            let raw = message.get_payload::<String>().ok()?;
            match decode::<Notification>(&raw) {
                Ok(notification) => Some(notification),
                Err(error) => {
                    tracing::warn!(%error, "dropping undecodable notification");
                    None
                }
            }
        });
        Ok(Box::pin(stream))
    }

    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = connection
            .get(self.session_meta_key(session_id))
            .await
            .map_err(|err| redis_error("read session", err))?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let meta: SessionMeta = decode(&raw)?;

        let encoded: Vec<String> = connection
            .lrange(self.session_messages_key(session_id), 0, -1)
            .await
            .map_err(|err| redis_error("read session messages", err))?;
        let messages: Vec<Message> =
            encoded.iter().map(|entry| decode(entry)).collect::<StoreResult<_>>()?;

        // Rebuild history: entries hosted elsewhere first, then one URL per
        // message this deployment stores.
        let base_url = meta.base_url.as_deref().unwrap_or_default();
        let mut history = meta.prefix_history.clone();
        history.extend((0..messages.len()).map(|index| message_url(base_url, session_id, index)));

        Ok(Some(SessionRecord {
            session: Session { id: meta.id, history, state: meta.state },
            messages,
        }))
    }

    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        let key = self.session_meta_key(session.id);
        let meta = SessionMeta {
            id: session.id,
            state: session.state.clone(),
            prefix_history: session.history.clone(),
            base_url: None,
        };

        let mut connection = self.connection.clone();
        // SET NX: whichever replica gets there first seeds the session; the
        // rest read what is already stored.
        let created: bool = connection
            .set_nx(&key, encode(&meta)?)
            .await
            .map_err(|err| redis_error("create session", err))?;
        if created {
            self.touch(&key).await?;
        }

        self.get_session(session.id).await?.ok_or_else(|| {
            Error::server_error(format!(
                "session {} vanished immediately after creation",
                session.id
            ))
        })
    }

    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let meta_key = self.session_meta_key(session_id);
        let messages_key = self.session_messages_key(session_id);
        let mut connection = self.connection.clone();

        // Ensure the session exists and knows the base URL its history links
        // resolve against.
        let raw: Option<String> =
            connection.get(&meta_key).await.map_err(|err| redis_error("read session", err))?;
        let mut meta: SessionMeta = match raw.as_deref() {
            Some(raw) => decode(raw)?,
            None => SessionMeta {
                id: session_id,
                state: None,
                prefix_history: Vec::new(),
                base_url: None,
            },
        };
        if meta.base_url.as_deref() != Some(base_url) {
            meta.base_url = Some(base_url.to_string());
            let _: () = connection
                .set(&meta_key, encode(&meta)?)
                .await
                .map_err(|err| redis_error("write session", err))?;
        }

        // A single RPUSH keeps concurrent appends from interleaving, and the
        // index each message lands at is exactly what its history URL uses.
        let encoded: Vec<String> =
            messages.iter().map(encode).collect::<StoreResult<Vec<String>>>()?;
        let _: () = connection
            .rpush(&messages_key, encoded)
            .await
            .map_err(|err| redis_error("append session messages", err))?;

        self.touch(&meta_key).await?;
        self.touch(&messages_key).await
    }

    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
        let mut connection = self.connection.clone();
        // Redis expires the key on its own, which is exactly the property we
        // need: a replica that dies stops renewing and the lease lapses without
        // anyone having to notice.
        let _: () = connection
            .pset_ex(self.lease_key(run_id), owner, lease_millis(ttl))
            .await
            .map_err(|err| redis_error("renew run lease", err))?;
        Ok(())
    }

    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        let mut connection = self.connection.clone();
        connection
            .get(self.lease_key(run_id))
            .await
            .map_err(|err| redis_error("read run lease", err))
    }

    async fn try_claim_lease(
        &self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> StoreResult<bool> {
        let mut connection = self.connection.clone();
        // SET NX PX is a single atomic operation, so exactly one claimant wins.
        let claimed: Option<String> = redis::cmd("SET")
            .arg(self.lease_key(run_id))
            .arg(owner)
            .arg("NX")
            .arg("PX")
            .arg(lease_millis(ttl))
            .query_async(&mut connection)
            .await
            .map_err(|err| redis_error("claim run lease", err))?;
        Ok(claimed.is_some())
    }

    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        let key = self.recovery_key(run_id);
        let mut connection = self.connection.clone();
        match record {
            Some(record) => {
                let payload = encode(record)?;
                match self.ttl_seconds() {
                    Some(seconds) => {
                        let _: () = connection
                            .set_ex(&key, payload, seconds as u64)
                            .await
                            .map_err(|err| redis_error("write recovery record", err))?;
                    }
                    None => {
                        let _: () = connection
                            .set(&key, payload)
                            .await
                            .map_err(|err| redis_error("write recovery record", err))?;
                    }
                }
            }
            None => {
                let _: () = connection
                    .del(&key)
                    .await
                    .map_err(|err| redis_error("clear recovery record", err))?;
            }
        }
        Ok(())
    }

    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = connection
            .get(self.recovery_key(run_id))
            .await
            .map_err(|err| redis_error("read recovery record", err))?;
        raw.as_deref().map(decode).transpose()
    }

    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        let mut connection = self.connection.clone();
        let _: () = connection
            .del(self.lease_key(run_id))
            .await
            .map_err(|err| redis_error("release run lease", err))?;
        Ok(())
    }

    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = connection
            .get(self.session_state_key(session_id))
            .await
            .map_err(|err| redis_error("read session state", err))?;
        raw.as_deref().map(decode).transpose()
    }

    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        let state_key = self.session_state_key(session_id);
        let meta_key = self.session_meta_key(session_id);
        let mut connection = self.connection.clone();

        let _: () = connection
            .set(&state_key, encode(&state)?)
            .await
            .map_err(|err| redis_error("write session state", err))?;

        // Record the link on the session, creating it if this is the first
        // thing written for it.
        let raw: Option<String> =
            connection.get(&meta_key).await.map_err(|err| redis_error("read session", err))?;
        let mut meta: SessionMeta = match raw.as_deref() {
            Some(raw) => decode(raw)?,
            None => SessionMeta {
                id: session_id,
                state: None,
                prefix_history: Vec::new(),
                base_url: None,
            },
        };
        meta.state = Some(state_url(base_url, session_id));
        let _: () = connection
            .set(&meta_key, encode(&meta)?)
            .await
            .map_err(|err| redis_error("write session", err))?;

        self.touch(&state_key).await?;
        self.touch(&meta_key).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PTTL` reports the remaining life of a key in milliseconds, which is the
    /// only way to check a lease's resolution without racing it. Sleeping past
    /// a boundary and asking who holds the lease would work, but the fractional
    /// second is the entire gap between right and wrong — a test built on it
    /// would have under half a second of margin on both sides and would fail on
    /// a loaded runner for reasons the code cannot fix.
    async fn remaining_millis(store: &RedisStore, run_id: RunId) -> i64 {
        let mut connection = store.connection.clone();
        redis::cmd("PTTL")
            .arg(store.lease_key(run_id))
            .query_async(&mut connection)
            .await
            .expect("PTTL")
    }

    /// Set `ACP_TEST_REDIS_URL` to run these. When it is set, an unreachable
    /// Redis fails rather than quietly skipping — a silent skip is how a
    /// backend goes untested for a release.
    async fn store() -> Option<RedisStore> {
        let url = std::env::var("ACP_TEST_REDIS_URL").ok()?;
        Some(
            RedisStore::connect(&url)
                .await
                .expect("ACP_TEST_REDIS_URL is set but Redis is unreachable"),
        )
    }

    #[test]
    fn a_lease_ttl_is_carried_at_the_resolution_it_was_given() {
        // The two ways `EX` got it wrong. Truncation is the dangerous one: a
        // lease that lapses early gets a live run reaped as abandoned.
        assert_eq!(lease_millis(Duration::from_millis(1500)), 1500);
        assert_eq!(lease_millis(Duration::from_millis(500)), 500);
        assert_eq!(lease_millis(Duration::from_secs(30)), 30_000);
    }

    /// Redis has no expiry shorter than one millisecond, and a lease of zero
    /// would be one already expired — which reads as a replica that never
    /// claimed the run rather than one that just did.
    #[test]
    fn a_zero_lease_still_lives_for_an_instant() {
        assert_eq!(lease_millis(Duration::ZERO), 1);
        assert_eq!(lease_millis(Duration::MAX), u64::MAX);
    }

    #[tokio::test]
    async fn a_renewed_lease_keeps_its_fractional_second() {
        let Some(store) = store().await else {
            eprintln!("skipping: set ACP_TEST_REDIS_URL to run the Redis lease tests");
            return;
        };
        let run_id = RunId::new();

        store.renew_lease(run_id, "replica-a", Duration::from_millis(1900)).await.unwrap();
        let remaining = remaining_millis(&store, run_id).await;
        assert!(
            remaining > 1000,
            "a 1900ms lease had {remaining}ms left, so it was truncated to whole seconds"
        );
        assert!(remaining <= 1900, "a 1900ms lease had {remaining}ms left");

        // And the other direction: a sub-second lease must not be rounded up to
        // a second, which is three times what a 300ms lease asked for.
        store.renew_lease(run_id, "replica-a", Duration::from_millis(300)).await.unwrap();
        let remaining = remaining_millis(&store, run_id).await;
        assert!(remaining <= 300, "a 300ms lease had {remaining}ms left, so it was rounded up");
    }

    /// `try_claim_lease` takes the same TTL and is a separate command, so it
    /// can drift from `renew_lease` without anything noticing.
    #[tokio::test]
    async fn a_claimed_lease_keeps_its_fractional_second() {
        let Some(store) = store().await else {
            eprintln!("skipping: set ACP_TEST_REDIS_URL to run the Redis lease tests");
            return;
        };
        let run_id = RunId::new();

        assert!(store
            .try_claim_lease(run_id, "replica-a", Duration::from_millis(1900))
            .await
            .unwrap());
        let remaining = remaining_millis(&store, run_id).await;
        assert!(
            remaining > 1000,
            "a 1900ms claim had {remaining}ms left, so it was truncated to whole seconds"
        );
        assert!(remaining <= 1900, "a 1900ms claim had {remaining}ms left");
    }
}
