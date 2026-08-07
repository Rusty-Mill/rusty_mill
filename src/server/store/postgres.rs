//! A Postgres-backed [`Store`], for durable multi-replica deployments.
//!
//! The same shape as [`RedisStore`](super::RedisStore) — every replica points
//! at one database and shares a view of every run — with two differences that
//! follow from the backend rather than from taste:
//!
//! - **Nothing expires on its own.** Redis is handed a TTL and forgets; Postgres
//!   keeps what it is given. That is usually the reason to choose it, so the
//!   default here is to keep everything and let retention be asked for.
//! - **Notifications carry a pointer, not a payload.** `NOTIFY` is capped at
//!   8000 bytes and an event carrying a base64 artifact will exceed it. See
//!   [`PostgresStore::subscribe`].

use std::time::Duration;

use futures_util::StreamExt;
use sqlx::postgres::{PgListener, PgPool, PgPoolOptions};
use sqlx::Row;

use crate::{
    server::store::{
        message_url, state_url, Notification, NotificationStream, RecoveryRecord, SessionRecord,
        Store, StoreResult,
    },
    types::{Error, Event, Message, Run, RunId, Session, SessionId},
};

/// Default prefix for the tables and channels this store owns.
pub const DEFAULT_TABLE_PREFIX: &str = "acp";

/// Largest notification payload sent inline.
///
/// Postgres caps a `NOTIFY` payload at 8000 bytes. The margin leaves room for
/// the envelope this store wraps around it.
const MAX_INLINE_PAYLOAD: usize = 7000;

/// Longest table prefix that survives into a channel name intact.
///
/// Postgres identifiers are capped at 63 bytes; a run id in its hyphen-less
/// form spends 32, plus one for the separator.
const MAX_CHANNEL_PREFIX: usize = 30;

/// What one [`sweep`](PostgresStore::sweep) removed.
///
/// Two numbers rather than one because they answer different questions. Runs
/// removed tracks the volume a retention window is holding back; sessions
/// removed is the one to watch, since a swept session is indistinguishable from
/// one that never existed and an agent that then writes to it silently starts
/// the conversation over.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Swept {
    /// Finished runs removed, with their events, leases, recovery records and
    /// signals.
    pub runs: u64,
    /// Sessions removed, with their message history and state documents.
    pub sessions: u64,
}

impl Swept {
    /// Whether the sweep found nothing to do.
    pub fn is_empty(&self) -> bool {
        self.runs == 0 && self.sessions == 0
    }
}

/// How a [`PostgresStore`] names its tables, and how long it keeps things.
#[derive(Debug, Clone)]
pub struct PostgresStoreConfig {
    /// Prefix for every table and channel, so several deployments can share one
    /// database.
    pub table_prefix: String,
    /// How long a finished run, or an untouched session, is kept before
    /// [`PostgresStore::sweep`] will remove it.
    ///
    /// `None` — the default — keeps everything forever. Postgres has no
    /// equivalent of a Redis TTL, so retention is a decision rather than
    /// something the store does for free, and unbounded history is usually the
    /// reason to pick this backend over Redis. Sweeping is never automatic:
    /// call [`PostgresStore::sweep`] from a job you control.
    ///
    /// One window for both, deliberately. A second knob would have to be
    /// justified by a deployment that wants its conversations kept on a
    /// different schedule from its runs, and until one turns up the extra
    /// setting is only another thing to get wrong. Both are measured from last
    /// write, so a session outlives the runs that fed it for exactly as long as
    /// a run outlives its own completion.
    pub retention: Option<Duration>,
    /// How much of one run's event log is kept, in bytes.
    ///
    /// A retention window bounds how *long* a log is kept, not how *much*: a
    /// single streaming run can fill a table well inside it. Matches
    /// [`InMemoryStore`](super::InMemoryStore)'s
    /// [`max_run_event_bytes`](super::InMemoryStore::max_run_event_bytes) so
    /// the backends agree on what a byte of log is, and measured with
    /// [`Event::approximate_size`] for the same reason.
    pub max_run_event_bytes: usize,
    /// Maximum pooled connections.
    pub max_connections: u32,
}

impl Default for PostgresStoreConfig {
    fn default() -> Self {
        Self {
            table_prefix: DEFAULT_TABLE_PREFIX.to_string(),
            retention: None,
            max_run_event_bytes: super::DEFAULT_MAX_RUN_EVENT_BYTES,
            max_connections: 10,
        }
    }
}

/// Keeps runs and sessions in Postgres, using `LISTEN`/`NOTIFY` for
/// notifications.
///
/// ```no_run
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// use rusty_acp::server::store::PostgresStore;
///
/// let store = PostgresStore::connect("postgres://localhost/acp").await?;
/// # Ok(())
/// # }
/// ```
///
/// [`connect`](PostgresStore::connect) creates the schema if it is not already
/// there, so a fresh database needs no migration step. The statements are all
/// `IF NOT EXISTS`, so several replicas starting at once is fine.
#[derive(Clone)]
pub struct PostgresStore {
    pool: PgPool,
    url: String,
    config: PostgresStoreConfig,
}

impl std::fmt::Debug for PostgresStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresStore").field("config", &self.config).finish_non_exhaustive()
    }
}

impl PostgresStore {
    /// Connect with the default configuration, creating the schema if needed.
    pub async fn connect(url: &str) -> StoreResult<Self> {
        Self::connect_with(url, PostgresStoreConfig::default()).await
    }

    /// Connect with an explicit configuration.
    pub async fn connect_with(url: &str, config: PostgresStoreConfig) -> StoreResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(url)
            .await
            .map_err(|err| pg_error("connect to Postgres", err))?;

        let store = Self { pool, url: url.to_string(), config };
        store.create_schema().await?;
        Ok(store)
    }

    /// The configuration in use.
    pub fn config(&self) -> &PostgresStoreConfig {
        &self.config
    }

    /// The connection pool, for callers who want to query the tables directly.
    ///
    /// Being able to ask "which runs failed today" is much of the point of
    /// choosing this backend, and that is a query rather than an API.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    fn table(&self, name: &str) -> String {
        format!("{}_{name}", self.config.table_prefix)
    }

    /// The channel a run's notifications are published on.
    ///
    /// Hyphens are stripped so the name needs no quoting. Postgres caps an
    /// identifier at 63 bytes and the run id spends 32 of them, so a prefix
    /// longer than [`MAX_CHANNEL_PREFIX`] is truncated *for the channel name
    /// only* — tables keep the prefix in full. Two deployments whose prefixes
    /// agree for that many characters would share channels, so keep them
    /// distinct early rather than late.
    fn channel(&self, run_id: RunId) -> String {
        let prefix = &self.config.table_prefix;
        let prefix = &prefix[..prefix.len().min(MAX_CHANNEL_PREFIX)];
        format!("{prefix}_{}", run_id.as_uuid().simple())
    }

    async fn create_schema(&self) -> StoreResult<()> {
        // `next_event` lives on the run rather than being derived from the log,
        // so `append_event` can take the row lock and hand out an index in one
        // statement. Deriving it with MAX(index)+1 would let two concurrent
        // appends compute the same one.
        let statements = [
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     run_id      uuid PRIMARY KEY,
                     run         jsonb NOT NULL,
                     next_event  bigint NOT NULL DEFAULT 0,
                     event_bytes bigint NOT NULL DEFAULT 0,
                     updated_at  timestamptz NOT NULL DEFAULT now()
                 )",
                self.table("runs")
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     run_id  uuid NOT NULL,
                     idx     bigint NOT NULL,
                     event   jsonb NOT NULL,
                     bytes   bigint NOT NULL DEFAULT 0,
                     PRIMARY KEY (run_id, idx)
                 )",
                self.table("events")
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     session_id      uuid PRIMARY KEY,
                     state_url       text,
                     prefix_history  jsonb NOT NULL DEFAULT '[]'::jsonb,
                     base_url        text,
                     next_message    bigint NOT NULL DEFAULT 0,
                     updated_at      timestamptz NOT NULL DEFAULT now()
                 )",
                self.table("sessions")
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     session_id  uuid NOT NULL,
                     idx         bigint NOT NULL,
                     message     jsonb NOT NULL,
                     PRIMARY KEY (session_id, idx)
                 )",
                self.table("session_messages")
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     session_id  uuid PRIMARY KEY,
                     state       jsonb NOT NULL
                 )",
                self.table("session_state")
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     run_id      uuid PRIMARY KEY,
                     owner       text NOT NULL,
                     expires_at  timestamptz NOT NULL
                 )",
                self.table("leases")
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     run_id  uuid PRIMARY KEY,
                     record  jsonb NOT NULL
                 )",
                self.table("recovery")
            ),
            // Control signals too large to ride on a NOTIFY payload. Events
            // never land here — they are already in the log, and the index on
            // the notification is enough to find them.
            format!(
                "CREATE TABLE IF NOT EXISTS {} (
                     id           bigserial PRIMARY KEY,
                     run_id       uuid NOT NULL,
                     notification jsonb NOT NULL,
                     created_at   timestamptz NOT NULL DEFAULT now()
                 )",
                self.table("signals")
            ),
        ];

        for statement in statements {
            sqlx::query(&statement)
                .execute(&self.pool)
                .await
                .map_err(|err| pg_error("create schema", err))?;
        }

        // `CREATE TABLE IF NOT EXISTS` does nothing to a table that already
        // exists, so a database created before the log bound would never get
        // these. Separate and idempotent, for the same reason the creates are:
        // several replicas may start at once.
        let columns = [
            format!(
                "ALTER TABLE {} ADD COLUMN IF NOT EXISTS event_bytes bigint NOT NULL DEFAULT 0",
                self.table("runs")
            ),
            format!(
                "ALTER TABLE {} ADD COLUMN IF NOT EXISTS bytes bigint NOT NULL DEFAULT 0",
                self.table("events")
            ),
        ];
        for statement in columns {
            sqlx::query(&statement)
                .execute(&self.pool)
                .await
                .map_err(|err| pg_error("add schema columns", err))?;
        }
        Ok(())
    }

    /// Drop the oldest events of a run until its log is back inside the bound.
    ///
    /// Run inside the appending transaction, so a reader never sees a log that
    /// has been trimmed but not yet accounted for.
    async fn trim_events(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        run_id: RunId,
    ) -> StoreResult<()> {
        let events = self.table("events");
        // The window sums backwards from the newest, so `running <= limit`
        // marks exactly the suffix that fits and `MIN(idx)` of it is the first
        // event to keep. `COALESCE` to the newest index covers the case where
        // not even one event fits: everything before it goes and the newest
        // stays, which is the same rule the other backends keep — a log that
        // dropped what it was just given could not serve a live tail.
        let deleted = sqlx::query(&format!(
            "WITH kept AS (
                 SELECT MIN(idx) AS first_kept FROM (
                     SELECT idx, SUM(bytes) OVER (ORDER BY idx DESC) AS running
                     FROM {events} WHERE run_id = $1
                 ) windowed WHERE running <= $2
             ),
             boundary AS (
                 SELECT COALESCE(
                     (SELECT first_kept FROM kept),
                     (SELECT MAX(idx) FROM {events} WHERE run_id = $1)
                 ) AS first_kept
             )
             DELETE FROM {events}
             WHERE run_id = $1 AND idx < (SELECT first_kept FROM boundary)
             RETURNING bytes",
            events = events,
        ))
        .bind(*run_id.as_uuid())
        .bind(self.config.max_run_event_bytes as i64)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|err| pg_error("trim event log", err))?;

        if deleted.is_empty() {
            return Ok(());
        }

        let freed: i64 =
            deleted.iter().map(|row| row.try_get::<i64, _>("bytes").unwrap_or(0)).sum();
        sqlx::query(&format!(
            "UPDATE {} SET event_bytes = GREATEST(event_bytes - $2, 0) WHERE run_id = $1",
            self.table("runs")
        ))
        .bind(*run_id.as_uuid())
        .bind(freed)
        .execute(&mut **transaction)
        .await
        .map_err(|err| pg_error("account for trimmed events", err))?;

        tracing::warn!(
            %run_id,
            dropped = deleted.len(),
            limit = self.config.max_run_event_bytes,
            "dropped the oldest events of a run past the log size bound"
        );
        Ok(())
    }

    /// Delete runs that finished longer ago than the configured retention —
    /// with the events, leases, recovery records and signals belonging to them
    /// — and sessions nothing has written to since the cutoff, with their
    /// message history and state documents.
    ///
    /// A no-op when no retention is configured, which is the default: deleting
    /// nothing is right for a backend usually chosen because Redis forgets too
    /// soon.
    ///
    /// # What makes a session stale
    ///
    /// **Last written to**, meaning adopted by a run, appended to, or having
    /// its state stored. Not last *read*: making a read a write would put a row
    /// lock on the sessions row in the path of every run that loads its own
    /// history, and turn a query that a read replica could serve into one it
    /// cannot. The cost is that a conversation being read but never added to
    /// ages out — which is the right answer anyway, since nobody is continuing
    /// it.
    ///
    /// This differs from [`InMemoryStore`](super::InMemoryStore), where a read
    /// does count as use. That store evicts under a *count* bound and has to
    /// choose a victim among live sessions, so it needs the finer signal; here
    /// the question is only whether a session is old.
    ///
    /// A session with a run still in flight is never collected, however old,
    /// so a sweep cannot take a conversation out from under a run that is about
    /// to append to it.
    ///
    /// An age-based rule was rejected: it would drop the long conversation
    /// still in progress in favour of the one nobody has opened in a year.
    pub async fn sweep(&self) -> StoreResult<Swept> {
        let Some(retention) = self.config.retention else {
            return Ok(Swept::default());
        };
        let cutoff = chrono::Utc::now()
            - chrono::Duration::from_std(retention).unwrap_or(chrono::Duration::zero());

        let runs = self.table("runs");
        let deleted = sqlx::query(&format!(
            "WITH stale AS (
                 DELETE FROM {runs}
                 WHERE updated_at < $1
                   AND run->>'status' IN ('completed', 'failed', 'cancelled')
                 RETURNING run_id
             ),
             cleaned_events AS (
                 DELETE FROM {events} WHERE run_id IN (SELECT run_id FROM stale)
             ),
             cleaned_leases AS (
                 DELETE FROM {leases} WHERE run_id IN (SELECT run_id FROM stale)
             ),
             cleaned_recovery AS (
                 DELETE FROM {recovery} WHERE run_id IN (SELECT run_id FROM stale)
             ),
             cleaned_signals AS (
                 DELETE FROM {signals} WHERE run_id IN (SELECT run_id FROM stale)
             )
             SELECT count(*) AS removed FROM stale",
            runs = runs,
            events = self.table("events"),
            leases = self.table("leases"),
            recovery = self.table("recovery"),
            signals = self.table("signals"),
        ))
        .bind(cutoff)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| pg_error("sweep expired runs", err))?;

        let sessions = self.table("sessions");
        // `NOT EXISTS` against non-terminal runs is what keeps a live
        // conversation. It stays correct as the runs table is swept around it,
        // because only terminal runs are ever swept — a run that would protect
        // a session is never one of the rows this statement's sibling removed.
        let collected = sqlx::query(&format!(
            "WITH stale AS (
                 DELETE FROM {sessions}
                 WHERE updated_at < $1
                   AND NOT EXISTS (
                       SELECT 1 FROM {runs}
                       WHERE run->>'session_id' = {sessions}.session_id::text
                         AND run->>'status' NOT IN ('completed', 'failed', 'cancelled')
                   )
                 RETURNING session_id
             ),
             cleaned_messages AS (
                 DELETE FROM {messages} WHERE session_id IN (SELECT session_id FROM stale)
             ),
             cleaned_state AS (
                 DELETE FROM {state} WHERE session_id IN (SELECT session_id FROM stale)
             )
             SELECT count(*) AS removed FROM stale",
            sessions = sessions,
            runs = runs,
            messages = self.table("session_messages"),
            state = self.table("session_state"),
        ))
        .bind(cutoff)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| pg_error("sweep expired sessions", err))?;

        let swept = Swept {
            runs: deleted.try_get::<i64, _>("removed").unwrap_or(0) as u64,
            sessions: collected.try_get::<i64, _>("removed").unwrap_or(0) as u64,
        };

        if swept.sessions > 0 {
            // Warned rather than logged at debug, and separately from the runs.
            // A collected session leaves nothing behind that says it existed,
            // so an agent handed that session id afterwards starts a fresh
            // conversation without any error to notice — the same silent
            // restart the in-memory store's eviction has, and worth the same
            // line in the log.
            tracing::warn!(
                sessions = swept.sessions,
                ?retention,
                "swept sessions past retention; their history and state are gone"
            );
        }
        Ok(swept)
    }
}

fn pg_error(action: &str, err: sqlx::Error) -> Error {
    Error::server_error(format!("failed to {action}: {err}"))
}

fn encode<T: serde::Serialize>(value: &T) -> StoreResult<serde_json::Value> {
    serde_json::to_value(value)
        .map_err(|err| Error::server_error(format!("failed to encode for Postgres: {err}")))
}

fn decode<T: serde::de::DeserializeOwned>(raw: serde_json::Value) -> StoreResult<T> {
    serde_json::from_value(raw)
        .map_err(|err| Error::server_error(format!("failed to decode from Postgres: {err}")))
}

/// What travels on a `NOTIFY`, in place of the notification itself.
///
/// Small enough to always fit the 8000-byte cap, whatever it points at.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "ref", rename_all = "snake_case")]
enum Pointer {
    /// An event, already in the log at this index.
    Event { index: u64 },
    /// A notification parked in the signals table.
    Signal { id: i64 },
    /// Small enough to travel whole. Boxed because it dwarfs the two pointer
    /// variants, which are the common case.
    Inline { notification: Box<Notification> },
}

#[async_trait::async_trait]
impl Store for PostgresStore {
    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        // `next_event` is deliberately not touched: the run snapshot is
        // overwritten constantly, and resetting the counter would hand out an
        // index the log already used.
        sqlx::query(&format!(
            "INSERT INTO {} (run_id, run, updated_at) VALUES ($1, $2, now())
             ON CONFLICT (run_id) DO UPDATE SET run = EXCLUDED.run, updated_at = now()",
            self.table("runs")
        ))
        .bind(*run.run_id.as_uuid())
        .bind(encode(run)?)
        .execute(&self.pool)
        .await
        .map_err(|err| pg_error("write run", err))?;
        Ok(())
    }

    async fn check_health(&self) -> StoreResult<()> {
        // Cheapest thing that proves a connection can be got from the pool and
        // used. A pool that is exhausted fails here, which is the point: it is
        // indistinguishable from an unreachable server to anything trying to
        // run an agent.
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|err| pg_error("health check", err))
    }

    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        let row = sqlx::query(&format!("SELECT run FROM {} WHERE run_id = $1", self.table("runs")))
            .bind(*run_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| pg_error("read run", err))?;

        match row {
            Some(row) => {
                let raw: serde_json::Value =
                    row.try_get("run").map_err(|err| pg_error("read run column", err))?;
                Ok(Some(decode(raw)?))
            }
            None => Ok(None),
        }
    }

    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
        let mut transaction =
            self.pool.begin().await.map_err(|err| pg_error("begin transaction", err))?;

        let size = event.approximate_size() as i64;

        // Taking the run's row lock is what makes the index unique: a
        // concurrent append blocks here rather than computing the same one.
        // The running byte total rides on the same statement, so accounting
        // for the log costs nothing on the path that does not trim.
        let row = sqlx::query(&format!(
            "UPDATE {} SET next_event = next_event + 1, event_bytes = event_bytes + $2
             WHERE run_id = $1 RETURNING next_event - 1 AS idx, event_bytes",
            self.table("runs")
        ))
        .bind(*run_id.as_uuid())
        .bind(size)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|err| pg_error("reserve event index", err))?;

        let Some(row) = row else {
            return Err(Error::not_found(format!("run {run_id} not found")));
        };
        let index: i64 = row.try_get("idx").map_err(|err| pg_error("read event index", err))?;
        let held: i64 = row.try_get("event_bytes").map_err(|err| pg_error("read log size", err))?;

        sqlx::query(&format!(
            "INSERT INTO {} (run_id, idx, event, bytes) VALUES ($1, $2, $3, $4)",
            self.table("events")
        ))
        .bind(*run_id.as_uuid())
        .bind(index)
        .bind(encode(event)?)
        .bind(size)
        .execute(&mut *transaction)
        .await
        .map_err(|err| pg_error("append event", err))?;

        if held > self.config.max_run_event_bytes as i64 {
            self.trim_events(&mut transaction, run_id).await?;
        }

        transaction.commit().await.map_err(|err| pg_error("commit event", err))?;
        Ok(index as u64)
    }

    async fn earliest_event(&self, run_id: RunId) -> StoreResult<u64> {
        // `MIN(idx)` is the answer directly: rows are deleted from the front,
        // and indices are absolute, so the lowest surviving one *is* where the
        // log now starts. Nothing to store and nothing that can disagree.
        let row: Option<(Option<i64>,)> = sqlx::query_as(&format!(
            "SELECT MIN(idx) FROM {} WHERE run_id = $1",
            self.table("events")
        ))
        .bind(*run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| pg_error("read the earliest event", err))?;
        Ok(row.and_then(|(idx,)| idx).unwrap_or(0).max(0) as u64)
    }

    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        self.events_from(run_id, 0).await
    }

    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        // Seeks on the primary key, so a reconnection costs the tail rather
        // than the whole run so far.
        let rows = sqlx::query(&format!(
            "SELECT event FROM {} WHERE run_id = $1 AND idx >= $2 ORDER BY idx",
            self.table("events")
        ))
        .bind(*run_id.as_uuid())
        .bind(from as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| pg_error("read events", err))?;

        rows.into_iter()
            .map(|row| {
                let raw: serde_json::Value =
                    row.try_get("event").map_err(|err| pg_error("read event column", err))?;
                decode(raw)
            })
            .collect()
    }

    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        let pointer = match &notification {
            // The event is in the log already, and the index says where. This
            // is the hot path — a streaming agent publishes per token — so it
            // costs no extra write however large the payload.
            Notification::Event { index: Some(index), .. } => Pointer::Event { index: *index },
            _ => {
                let encoded = encode(&notification)?;
                if serde_json::to_string(&encoded).map(|json| json.len()).unwrap_or(usize::MAX)
                    <= MAX_INLINE_PAYLOAD
                {
                    Pointer::Inline { notification: Box::new(notification) }
                } else {
                    // A resume payload can be arbitrarily large. Park it and
                    // send its id instead of failing the publish.
                    let row = sqlx::query(&format!(
                        "INSERT INTO {} (run_id, notification) VALUES ($1, $2) RETURNING id",
                        self.table("signals")
                    ))
                    .bind(*run_id.as_uuid())
                    .bind(encoded)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|err| pg_error("park oversized notification", err))?;
                    let id: i64 =
                        row.try_get("id").map_err(|err| pg_error("read signal id", err))?;
                    Pointer::Signal { id }
                }
            }
        };

        let payload = serde_json::to_string(&pointer)
            .map_err(|err| Error::server_error(format!("failed to encode pointer: {err}")))?;

        // pg_notify takes the channel as a parameter, so the name needs no
        // quoting and cannot be injected into.
        // No subscribers is not an error: the event log is the durable record.
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(self.channel(run_id))
            .bind(payload)
            .execute(&self.pool)
            .await
            .map_err(|err| pg_error("publish notification", err))?;
        Ok(())
    }

    /// Subscribe to a run's channel.
    ///
    /// `NOTIFY` payloads are capped at 8000 bytes, which an event carrying a
    /// base64 artifact will exceed, so what travels is a small pointer and the
    /// subscriber reads the row. Events cost nothing extra — they are already
    /// in the log, and [`Notification::Event`] carries the index that finds
    /// them.
    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        let mut listener = PgListener::connect(&self.url)
            .await
            .map_err(|err| pg_error("open Postgres listener", err))?;
        // Returns only once Postgres has acknowledged the LISTEN, so a caller
        // can subscribe and then act without missing what its own action
        // triggers.
        listener
            .listen(&self.channel(run_id))
            .await
            .map_err(|err| pg_error("listen on run channel", err))?;

        let store = self.clone();
        let stream = listener.into_stream().filter_map(move |message| {
            let store = store.clone();
            async move {
                let message = message.ok()?;
                let pointer: Pointer = match serde_json::from_str(message.payload()) {
                    Ok(pointer) => pointer,
                    Err(error) => {
                        tracing::warn!(%error, "dropping undecodable notification pointer");
                        return None;
                    }
                };
                store.resolve(run_id, pointer).await
            }
        });
        Ok(Box::pin(stream))
    }

    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        let row = sqlx::query(&format!(
            "SELECT state_url, prefix_history, base_url FROM {} WHERE session_id = $1",
            self.table("sessions")
        ))
        .bind(*session_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| pg_error("read session", err))?;

        let Some(row) = row else {
            return Ok(None);
        };
        let state: Option<String> =
            row.try_get("state_url").map_err(|err| pg_error("read session state", err))?;
        let prefix_history: serde_json::Value =
            row.try_get("prefix_history").map_err(|err| pg_error("read session history", err))?;
        let base_url: Option<String> =
            row.try_get("base_url").map_err(|err| pg_error("read session base url", err))?;

        let messages = self.session_messages(session_id).await?;

        // History is rebuilt rather than stored: entries hosted elsewhere
        // first, then one URL per message this deployment holds. The URL for
        // message `i` is a pure function of `i`, which is what lets an append
        // be a plain insert.
        let base_url = base_url.unwrap_or_default();
        let mut history: Vec<String> = decode(prefix_history)?;
        history.extend((0..messages.len()).map(|index| message_url(&base_url, session_id, index)));

        Ok(Some(SessionRecord { session: Session { id: session_id, history, state }, messages }))
    }

    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        // Whichever replica gets there first seeds the session; the rest read
        // what is already stored.
        //
        // The conflict branch touches `updated_at` and nothing else. This is a
        // run adopting the session, which is the moment `sweep` has to stop
        // treating it as idle — otherwise a conversation quiet for longer than
        // the retention window could be collected between the run picking it up
        // and the run appending its output, and the history would come back
        // empty with nothing raised.
        sqlx::query(&format!(
            "INSERT INTO {} (session_id, state_url, prefix_history) VALUES ($1, $2, $3)
             ON CONFLICT (session_id) DO UPDATE SET updated_at = now()",
            self.table("sessions")
        ))
        .bind(*session.id.as_uuid())
        .bind(session.state.clone())
        .bind(encode(&session.history)?)
        .execute(&self.pool)
        .await
        .map_err(|err| pg_error("create session", err))?;

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

        let mut transaction =
            self.pool.begin().await.map_err(|err| pg_error("begin transaction", err))?;

        // One statement reserves the whole block of indices under the session's
        // row lock, so two replicas appending at once cannot interleave or land
        // on the same index — the invariant the trait requires.
        let row = sqlx::query(&format!(
            "INSERT INTO {} (session_id, base_url, next_message, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (session_id) DO UPDATE
               SET next_message = {sessions}.next_message + $3,
                   base_url = EXCLUDED.base_url,
                   updated_at = now()
             RETURNING next_message - $3 AS first_index",
            self.table("sessions"),
            sessions = self.table("sessions"),
        ))
        .bind(*session_id.as_uuid())
        .bind(base_url)
        .bind(messages.len() as i64)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|err| pg_error("reserve session message indices", err))?;

        let first: i64 =
            row.try_get("first_index").map_err(|err| pg_error("read message index", err))?;

        for (offset, message) in messages.iter().enumerate() {
            sqlx::query(&format!(
                "INSERT INTO {} (session_id, idx, message) VALUES ($1, $2, $3)",
                self.table("session_messages")
            ))
            .bind(*session_id.as_uuid())
            .bind(first + offset as i64)
            .bind(encode(message)?)
            .execute(&mut *transaction)
            .await
            .map_err(|err| pg_error("append session message", err))?;
        }

        transaction.commit().await.map_err(|err| pg_error("commit session messages", err))
    }

    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        let row = sqlx::query(&format!(
            "SELECT state FROM {} WHERE session_id = $1",
            self.table("session_state")
        ))
        .bind(*session_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| pg_error("read session state", err))?;

        match row {
            Some(row) => Ok(Some(
                row.try_get("state").map_err(|err| pg_error("read session state column", err))?,
            )),
            None => Ok(None),
        }
    }

    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        let mut transaction =
            self.pool.begin().await.map_err(|err| pg_error("begin transaction", err))?;

        sqlx::query(&format!(
            "INSERT INTO {} (session_id, state) VALUES ($1, $2)
             ON CONFLICT (session_id) DO UPDATE SET state = EXCLUDED.state",
            self.table("session_state")
        ))
        .bind(*session_id.as_uuid())
        .bind(&state)
        .execute(&mut *transaction)
        .await
        .map_err(|err| pg_error("write session state", err))?;

        // ACP models state as a link rather than inline content, so the session
        // has to point at the document.
        sqlx::query(&format!(
            "INSERT INTO {} (session_id, state_url, updated_at) VALUES ($1, $2, now())
             ON CONFLICT (session_id) DO UPDATE SET state_url = EXCLUDED.state_url, updated_at = now()",
            self.table("sessions")
        ))
        .bind(*session_id.as_uuid())
        .bind(state_url(base_url, session_id))
        .execute(&mut *transaction)
        .await
        .map_err(|err| pg_error("link session state", err))?;

        transaction.commit().await.map_err(|err| pg_error("commit session state", err))
    }

    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
        // Unconditional: the executing replica is the only writer, so there is
        // no other claimant to lose a race with.
        sqlx::query(&format!(
            "INSERT INTO {} (run_id, owner, expires_at) VALUES ($1, $2, now() + $3)
             ON CONFLICT (run_id) DO UPDATE
               SET owner = EXCLUDED.owner, expires_at = EXCLUDED.expires_at",
            self.table("leases")
        ))
        .bind(*run_id.as_uuid())
        .bind(owner)
        .bind(pg_interval(ttl))
        .execute(&self.pool)
        .await
        .map_err(|err| pg_error("renew run lease", err))?;
        Ok(())
    }

    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        // Expiry is a column rather than something the database enforces, so
        // "has it lapsed" is part of the read. A lapsed row is left in place;
        // `sweep` removes it with the run.
        let row = sqlx::query(&format!(
            "SELECT owner FROM {} WHERE run_id = $1 AND expires_at > now()",
            self.table("leases")
        ))
        .bind(*run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| pg_error("read run lease", err))?;

        match row {
            Some(row) => {
                Ok(Some(row.try_get("owner").map_err(|err| pg_error("read lease owner", err))?))
            }
            None => Ok(None),
        }
    }

    async fn try_claim_lease(
        &self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> StoreResult<bool> {
        // Atomic by construction: the conditional DO UPDATE either takes the
        // lease or does nothing, in one statement. Exactly one replica can be
        // told it won, which is what stops two of them recovering the same
        // abandoned run.
        let row = sqlx::query(&format!(
            "INSERT INTO {leases} (run_id, owner, expires_at) VALUES ($1, $2, now() + $3)
             ON CONFLICT (run_id) DO UPDATE
               SET owner = EXCLUDED.owner, expires_at = EXCLUDED.expires_at
               WHERE {leases}.expires_at <= now()
             RETURNING run_id",
            leases = self.table("leases")
        ))
        .bind(*run_id.as_uuid())
        .bind(owner)
        .bind(pg_interval(ttl))
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| pg_error("claim run lease", err))?;

        Ok(row.is_some())
    }

    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        match record {
            Some(record) => {
                sqlx::query(&format!(
                    "INSERT INTO {} (run_id, record) VALUES ($1, $2)
                     ON CONFLICT (run_id) DO UPDATE SET record = EXCLUDED.record",
                    self.table("recovery")
                ))
                .bind(*run_id.as_uuid())
                .bind(encode(record)?)
                .execute(&self.pool)
                .await
                .map_err(|err| pg_error("write recovery record", err))?;
            }
            None => {
                sqlx::query(&format!("DELETE FROM {} WHERE run_id = $1", self.table("recovery")))
                    .bind(*run_id.as_uuid())
                    .execute(&self.pool)
                    .await
                    .map_err(|err| pg_error("clear recovery record", err))?;
            }
        }
        Ok(())
    }

    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        let row = sqlx::query(&format!(
            "SELECT record FROM {} WHERE run_id = $1",
            self.table("recovery")
        ))
        .bind(*run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| pg_error("read recovery record", err))?;

        match row {
            Some(row) => {
                let raw: serde_json::Value =
                    row.try_get("record").map_err(|err| pg_error("read recovery column", err))?;
                Ok(Some(decode(raw)?))
            }
            None => Ok(None),
        }
    }

    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        sqlx::query(&format!("DELETE FROM {} WHERE run_id = $1", self.table("leases")))
            .bind(*run_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|err| pg_error("release run lease", err))?;
        Ok(())
    }
}

impl PostgresStore {
    /// Turn a notification pointer back into the notification it stands for.
    async fn resolve(&self, run_id: RunId, pointer: Pointer) -> Option<Notification> {
        match pointer {
            Pointer::Inline { notification } => Some(*notification),
            Pointer::Event { index } => match self.events_from(run_id, index).await {
                Ok(events) => {
                    events.into_iter().next().map(|event| Notification::event_at(index, event))
                }
                Err(error) => {
                    tracing::warn!(%error, %run_id, index, "failed to read a notified event");
                    None
                }
            },
            Pointer::Signal { id } => {
                let row = sqlx::query(&format!(
                    "SELECT notification FROM {} WHERE id = $1",
                    self.table("signals")
                ))
                .bind(id)
                .fetch_optional(&self.pool)
                .await;

                match row {
                    Ok(Some(row)) => {
                        let raw: serde_json::Value = row.try_get("notification").ok()?;
                        decode(raw).ok()
                    }
                    Ok(None) => None,
                    Err(error) => {
                        tracing::warn!(%error, id, "failed to read a parked notification");
                        None
                    }
                }
            }
        }
    }

    async fn session_messages(&self, session_id: SessionId) -> StoreResult<Vec<Message>> {
        let rows = sqlx::query(&format!(
            "SELECT message FROM {} WHERE session_id = $1 ORDER BY idx",
            self.table("session_messages")
        ))
        .bind(*session_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|err| pg_error("read session messages", err))?;

        rows.into_iter()
            .map(|row| {
                let raw: serde_json::Value =
                    row.try_get("message").map_err(|err| pg_error("read message column", err))?;
                decode(raw)
            })
            .collect()
    }
}

/// A `Duration` as the interval Postgres wants for `now() + $n`.
fn pg_interval(duration: Duration) -> sqlx::postgres::types::PgInterval {
    sqlx::postgres::types::PgInterval {
        months: 0,
        days: 0,
        microseconds: duration.as_micros().min(i64::MAX as u128) as i64,
    }
}
