//! The transactional outbox pattern -- the Rust port of
//! `meshed.infrastructure.outbox` (SDK-055..064).
//!
//! [`OutboxEntry`] (SDK-055) and [`write_outbox_entry`] (SDK-056):
//! plain data plus a SQLite insert that deliberately doesn't manage
//! its own transaction boundary, so a caller can wrap it and their own
//! business-entity write in one atomic `rusqlite::Transaction` -- the
//! core invariant this pattern exists for (see [`write_outbox_entry`]'s
//! own doc for how).
//!
//! [`OutboxRelay`] (SDK-057..064) is built on `rusty_kafka`'s
//! `Produce` support (the same gap that used to block this and
//! `rusty-meshed-observability::slo`'s `SLOViolationPublisher`, tracked
//! on GitHub issue #87). It's a background daemon thread in the
//! source, driven by `threading.Thread` + a persistent
//! `confluent_kafka.Producer`; this port keeps the daemon-thread shape
//! (`std::thread::spawn`, a `stop()` that signals and joins with a
//! bounded wait) but swaps the persistent producer for a fresh
//! `rusty_kafka::KafkaClient<TcpStream>` connected only when there's
//! actually something pending -- seeded by this crate's per-call-
//! connection convention (see `rusty-meshed-registry::app::AppState`'s
//! own doc for where that convention comes from) applied to the DB
//! side already, and extended to the Kafka side here since
//! `rusty_kafka::KafkaClient` has no lazy-connect-on-first-produce
//! equivalent to `librdkafka`'s. [`relay_pending`] -- the testable
//! core, generic over any `AsyncRead + AsyncWrite` stream the same way
//! `SLOViolationPublisher`/`SLOMonitor` are -- is exercised directly by
//! this module's tests against a fake broker; [`OutboxRelay`]'s own
//! thread-lifecycle/real-TCP-connect plumbing isn't (same reason
//! `KafkaClient::connect` itself isn't unit tested anywhere in this
//! crate family: no live broker in this environment).

use rusty_err::Error;
use rusty_kafka::protocol::produce::{
    ProducePartitionRequest, ProduceRequest, ProduceTopicRequest,
};
use rusty_kafka::record_batch::Record;
use rusty_kafka::{ClientError, KafkaClient};
use rusty_sqlite::rusqlite::{params, Connection, Result as SqlResult};
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A pending outbox event waiting to be relayed to Kafka (SDK-055).
/// Written atomically with its associated business data via
/// [`write_outbox_entry`] plus the caller's own write, inside one
/// `rusqlite` transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEntry {
    pub id: i64,
    pub event_type: String,
    pub topic: String,
    /// JSON-encoded event payload.
    pub payload: String,
    /// JSON-encoded Kafka message headers; `"{}"` if none were given.
    pub headers: String,
    pub created_at: String,
    /// `None` while pending; set by [`OutboxRelay`] after a successful
    /// publish.
    pub published_at: Option<String>,
}

/// Creates the `outbox_entries` table if it doesn't already exist.
pub fn ensure_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS outbox_entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type TEXT NOT NULL,
            topic TEXT NOT NULL,
            payload TEXT NOT NULL,
            headers TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            published_at TEXT
        );",
    )
}

/// Inserts a pending outbox entry (SDK-056). Does **not** commit --
/// same as the source's `session.add()` (staged, not yet written until
/// `session.commit()`), the caller is responsible for committing so
/// this write lands in the same atomic transaction as any associated
/// business data. Concretely: call this with a `&rusqlite::Transaction`
/// (it derefs to `&Connection`) alongside your own business write,
/// then call `.commit()` once -- a plain `&Connection` outside an
/// explicit transaction works too, but then this insert commits on its
/// own the moment this function returns (SQLite's normal autocommit
/// behavior), which is fine for a standalone outbox write but forfeits
/// the atomicity guarantee this pattern exists for.
pub fn write_outbox_entry(
    conn: &Connection,
    event_type: &str,
    topic: &str,
    payload: &rusty_json::Value,
    headers: Option<&rusty_json::Value>,
) -> SqlResult<OutboxEntry> {
    let created_at = now_iso();
    let payload_json = rusty_json::to_string(payload).unwrap_or_else(|_| "null".to_string());
    let headers_json = headers
        .map(|h| rusty_json::to_string(h).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "{}".to_string());

    conn.execute(
        "INSERT INTO outbox_entries (event_type, topic, payload, headers, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![event_type, topic, payload_json, headers_json, created_at],
    )?;
    Ok(OutboxEntry {
        id: conn.last_insert_rowid(),
        event_type: event_type.to_string(),
        topic: topic.to_string(),
        payload: payload_json,
        headers: headers_json,
        created_at,
        published_at: None,
    })
}

/// Every outbox entry, oldest first -- what `demo_outbox`'s
/// no-relay path prints to show the atomic write actually landed.
pub fn fetch_all(conn: &Connection) -> SqlResult<Vec<OutboxEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, topic, payload, headers, created_at, published_at \
         FROM outbox_entries ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(OutboxEntry {
            id: row.get(0)?,
            event_type: row.get(1)?,
            topic: row.get(2)?,
            payload: row.get(3)?,
            headers: row.get(4)?,
            created_at: row.get(5)?,
            published_at: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Errors from one [`relay_pending`] batch. Never seen by
/// [`OutboxRelay`]'s own background thread, which deliberately
/// discards this (see [`relay_pending`]'s own doc for why) -- surfaced
/// here for tests and any other direct caller.
#[derive(Debug, Error)]
pub enum RelayError {
    /// The underlying Kafka request itself failed (connection, framing,
    /// correlation mismatch, ...).
    #[error("Kafka client error: {0}")]
    Kafka(#[from] ClientError),
    /// The broker's response didn't include a result for the
    /// topic/partition produced to.
    #[error("no result for the requested topic/partition in the broker's response")]
    MissingPartitionResult,
    /// The broker returned a non-zero error code for the produce
    /// (e.g. `UNKNOWN_TOPIC_OR_PARTITION`).
    #[error("broker returned Kafka error code {0}")]
    KafkaErrorCode(i16),
    /// The outbox database read/write itself failed.
    #[error("outbox database error: {0}")]
    Sql(String),
}

/// Selects up to `limit` pending entries (`published_at IS NULL`,
/// oldest first), produces each to its own topic at partition 0 (no
/// key, matching the source's `producer.produce(topic=..., value=...,
/// headers=...)` -- no `key=` argument given), and marks each
/// published, all as one transaction (SDK-057..064 except
/// `OutboxRelay`'s own thread lifecycle, which stays in
/// [`OutboxRelay::start`]/[`OutboxRelay::stop`]).
///
/// Partition 0 always -- same single-partition-topic assumption
/// documented on `rusty-meshed-observability::slo::
/// SLOViolationPublisher::publish` (this client has no
/// `Metadata`-based partitioner), extended here from one fixed
/// governance topic to whatever topic each entry names, since that's
/// still the only partition any topic in this platform's local-dev
/// deployment has.
///
/// One `rusqlite::Transaction` for the whole batch (SDK-063): each
/// successfully-produced entry's `published_at` update is staged on
/// `conn`'s transaction as the loop goes, but a failure partway
/// through returns immediately (SDK-064) without committing, which
/// rolls back every update staged so far in this call -- so a batch
/// either fully lands or, on the first Kafka error, none of it does
/// (the already-produced-but-now-rolled-back entries get republished
/// on the next successful batch, since Kafka gave no way to "undo" the
/// produce -- the source's own documented "at-least-once, no dedup").
/// This matches the source's actual code path here more closely than
/// its own docstring does: `_relay_pending`'s docstring promises "a
/// failed produce will leave the entry pending so it is retried on the
/// next poll cycle", but nothing in `_relay_loop`/`_relay_pending`
/// actually catches an exception from `producer.produce()` -- an
/// uncaught exception there would unwind out of `_relay_pending`
/// entirely, past the `while` loop in `_relay_loop`, silently ending
/// the background thread's `run()` on the very first failure and never
/// retrying again. [`OutboxRelay`]'s own background thread implements
/// the *documented* retry-next-cycle intent instead of that latent bug
/// -- see its own doc.
pub async fn relay_pending<S: AsyncRead + AsyncWrite + Unpin + Send>(
    conn: &mut Connection,
    client: &mut KafkaClient<S>,
    limit: i64,
) -> Result<usize, RelayError> {
    let tx = conn
        .transaction()
        .map_err(|err| RelayError::Sql(err.to_string()))?;
    let pending = fetch_pending(&tx, limit).map_err(|err| RelayError::Sql(err.to_string()))?;

    let mut published = 0;
    for entry in &pending {
        publish_one(client, entry).await?;
        mark_published(&tx, entry.id).map_err(|err| RelayError::Sql(err.to_string()))?;
        published += 1;
    }

    tx.commit()
        .map_err(|err| RelayError::Sql(err.to_string()))?;
    Ok(published)
}

async fn publish_one<S: AsyncRead + AsyncWrite + Unpin + Send>(
    client: &mut KafkaClient<S>,
    entry: &OutboxEntry,
) -> Result<(), RelayError> {
    let request = ProduceRequest {
        acks: -1,
        timeout_ms: 5000,
        base_timestamp_ms: now_millis(),
        topics: vec![ProduceTopicRequest {
            name: entry.topic.clone(),
            partitions: vec![ProducePartitionRequest {
                partition_index: 0,
                records: vec![Record {
                    key: None,
                    value: Some(entry.payload.clone().into_bytes()),
                    headers: parse_headers(&entry.headers),
                }],
            }],
        }],
    };
    let response = client.produce(&request).await?;
    let result = response
        .topics
        .first()
        .and_then(|t| t.partitions.first())
        .ok_or(RelayError::MissingPartitionResult)?;
    if result.error_code != 0 {
        return Err(RelayError::KafkaErrorCode(result.error_code));
    }
    Ok(())
}

/// `entry.headers`'s JSON object as Kafka wire headers. A string value
/// becomes its own UTF-8 bytes; anything else (a number, `null`, a
/// nested object -- `write_outbox_entry` accepts any JSON value as a
/// header, not just strings) falls back to that value's own JSON
/// encoding, so nothing here panics or silently drops a non-string
/// header. Malformed JSON (shouldn't happen -- only this module ever
/// writes this column) or a non-object value produces no headers
/// rather than failing the whole produce over it.
fn parse_headers(headers_json: &str) -> Vec<(String, Option<Vec<u8>>)> {
    let Ok(value) = rusty_json::from_str::<rusty_json::Value>(headers_json) else {
        return Vec::new();
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .map(|(key, value)| {
            let bytes = match value.as_str() {
                Some(s) => s.as_bytes().to_vec(),
                None => rusty_json::to_string(value)
                    .unwrap_or_default()
                    .into_bytes(),
            };
            (key.clone(), Some(bytes))
        })
        .collect()
}

fn fetch_pending(conn: &Connection, limit: i64) -> SqlResult<Vec<OutboxEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, topic, payload, headers, created_at, published_at \
         FROM outbox_entries WHERE published_at IS NULL ORDER BY id ASC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(OutboxEntry {
            id: row.get(0)?,
            event_type: row.get(1)?,
            topic: row.get(2)?,
            payload: row.get(3)?,
            headers: row.get(4)?,
            created_at: row.get(5)?,
            published_at: row.get(6)?,
        })
    })?;
    rows.collect()
}

fn has_pending(conn: &Connection) -> SqlResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM outbox_entries WHERE published_at IS NULL)",
        [],
        |row| row.get(0),
    )
}

fn mark_published(conn: &Connection, id: i64) -> SqlResult<()> {
    let published_at = now_iso();
    conn.execute(
        "UPDATE outbox_entries SET published_at = ?1 WHERE id = ?2",
        params![published_at, id],
    )?;
    Ok(())
}

/// Background daemon thread that relays pending [`OutboxEntry`] rows to
/// Kafka (SDK-057..064) -- `OutboxRelay.__init__`/`start`/`stop` from
/// the source, ported to `std::thread` (see this module's own doc for
/// the `confluent_kafka.Producer` → per-batch `KafkaClient` swap).
pub struct OutboxRelay {
    db_path: String,
    bootstrap_servers: String,
    stop_tx: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl OutboxRelay {
    /// Seconds between polling cycles (SDK-057).
    pub const POLL_INTERVAL_SECONDS: f64 = 2.0;

    /// `db_path` is a plain SQLite file path, not a SQLAlchemy URL
    /// (`"sqlite:///meshed_registry.db"`) -- same translation this
    /// crate family applies everywhere else a source `db_url` becomes
    /// a `rusqlite::Connection` target (SDK-058).
    pub fn new(db_path: impl Into<String>, bootstrap_servers: impl Into<String>) -> Self {
        OutboxRelay {
            db_path: db_path.into(),
            bootstrap_servers: bootstrap_servers.into(),
            stop_tx: None,
            thread: None,
        }
    }

    /// Launches the relay as a daemon background thread (SDK-059).
    /// Calling this twice, like the source, just spawns a second
    /// thread racing the first against the same database -- neither
    /// side guards against it.
    pub fn start(&mut self) {
        let (stop_tx, stop_rx) = mpsc::channel();
        let db_path = self.db_path.clone();
        let bootstrap_servers = self.bootstrap_servers.clone();
        let handle = thread::Builder::new()
            .name("rusty_meshed_outbox_relay".to_string())
            .spawn(move || relay_loop(&db_path, &bootstrap_servers, &stop_rx))
            .expect("failed to spawn outbox relay thread");
        self.stop_tx = Some(stop_tx);
        self.thread = Some(handle);
    }

    /// Signals the relay to stop and waits up to 5 seconds for it to
    /// finish its current cycle (SDK-060). `std::thread::JoinHandle`
    /// has no timed `join` the way Python's `Thread.join(timeout=...)`
    /// does, so this hands the join itself to a throwaway watcher
    /// thread and waits on *that* with a timeout instead -- same
    /// best-effort semantics either language gives: a slow cycle means
    /// this returns before the relay thread has actually exited, not
    /// that it's forcibly killed (neither runtime can do that to a
    /// thread).
    pub fn stop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        let Some(handle) = self.thread.take() else {
            return;
        };
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = handle.join();
            let _ = done_tx.send(());
        });
        let _ = done_rx.recv_timeout(Duration::from_secs(5));
    }
}

/// The relay thread's body (SDK-061): poll, then wait out
/// `POLL_INTERVAL_SECONDS` unless `stop_rx` fires first, repeat until
/// it does (a send or a disconnect -- `OutboxRelay` dropped without an
/// explicit [`OutboxRelay::stop`] stops the relay the same way `stop()`
/// would, rather than leaking a thread nothing can reach again).
fn relay_loop(db_path: &str, bootstrap_servers: &str, stop_rx: &mpsc::Receiver<()>) {
    let Ok(runtime) = rusty_tokio::Builder::new_current_thread().build() else {
        return;
    };
    loop {
        runtime.block_on(poll_once(db_path, bootstrap_servers));
        match stop_rx.recv_timeout(Duration::from_secs_f64(OutboxRelay::POLL_INTERVAL_SECONDS)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// One polling cycle: open a fresh connection (this crate's per-call-
/// connection convention), skip Kafka entirely if nothing is pending,
/// otherwise connect and hand off to [`relay_pending`]. Every failure
/// here -- can't open the database, can't connect, a
/// [`RelayError`] from `relay_pending` -- is deliberately swallowed:
/// this is the one place that implements SDK-064's documented
/// retry-next-cycle intent (see [`relay_pending`]'s own doc for why
/// that's not simply "port the source's exception handling") by making
/// sure nothing thrown from a single cycle ever reaches
/// [`relay_loop`] and ends the background thread.
async fn poll_once(db_path: &str, bootstrap_servers: &str) {
    let Ok(mut conn) = Connection::open(db_path) else {
        return;
    };
    let Ok(true) = has_pending(&conn) else {
        return;
    };
    let Ok(mut client) = KafkaClient::<TcpStream>::connect(
        bootstrap_servers,
        Some("rusty_meshed_outbox_relay".to_string()),
    )
    .await
    else {
        return;
    };
    let _ = relay_pending(&mut conn, &mut client, 100).await;
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// A minimal RFC 3339 UTC "now" formatter -- same hand-rolled
/// civil-from-days algorithm duplicated elsewhere in this crate family.
fn now_iso() -> String {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let mut days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_json::json;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn write_outbox_entry_defaults_headers_to_an_empty_object() {
        let conn = seeded_connection();
        let payload = json!({"a": 1});
        let entry = write_outbox_entry(&conn, "PersonnelAssigned", "t", &payload, None).unwrap();
        assert_eq!(entry.headers, "{}");
        assert_eq!(entry.payload, "{\"a\":1}");
        assert!(entry.published_at.is_none());
        assert!(entry.id > 0);
    }

    #[test]
    fn write_outbox_entry_serializes_the_given_headers() {
        let conn = seeded_connection();
        let payload = json!({});
        let headers = json!({"source": "demo", "version": "1"});
        let entry =
            write_outbox_entry(&conn, "PersonnelAssigned", "t", &payload, Some(&headers)).unwrap();
        let parsed: rusty_json::Value = rusty_json::from_str(&entry.headers).unwrap();
        assert_eq!(parsed.get("source").unwrap().as_str(), Some("demo"));
    }

    #[test]
    fn fetch_all_returns_entries_oldest_first() {
        let conn = seeded_connection();
        let payload = json!({});
        write_outbox_entry(&conn, "A", "t1", &payload, None).unwrap();
        write_outbox_entry(&conn, "B", "t2", &payload, None).unwrap();

        let entries = fetch_all(&conn).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].event_type, "A");
        assert_eq!(entries[1].event_type, "B");
    }

    #[test]
    fn a_rolled_back_transaction_leaves_no_entry_committed() {
        // Demonstrates the atomicity guarantee write_outbox_entry's own
        // doc describes: nothing lands until the wrapping transaction
        // commits, and a rollback undoes the insert entirely.
        let mut conn = seeded_connection();
        let payload = json!({});
        {
            let tx = conn.transaction().unwrap();
            write_outbox_entry(&tx, "A", "t1", &payload, None).unwrap();
            // No commit -- tx is dropped here, rolling back.
        }
        assert!(fetch_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn a_committed_transaction_persists_the_entry() {
        let mut conn = seeded_connection();
        let payload = json!({});
        {
            let tx = conn.transaction().unwrap();
            write_outbox_entry(&tx, "A", "t1", &payload, None).unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(fetch_all(&conn).unwrap().len(), 1);
    }

    use rusty_kafka::protocol::produce::{
        ProducePartitionResponse, ProduceRequest as DecodedProduceRequest, ProduceResponse,
        ProduceTopicResponse,
    };
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_tokio::io::duplex;
    use rusty_wire::Writer;

    async fn respond_to_produce(
        peer: &mut (impl rusty_tokio::io::AsyncRead + rusty_tokio::io::AsyncWrite + Unpin + Send),
        topic: &str,
        error_code: i16,
    ) -> DecodedProduceRequest {
        let (header, body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, rusty_kafka::protocol::api_key::PRODUCE);
        let mut reader = rusty_wire::Reader::new(&body);
        let request = DecodedProduceRequest::decode(&mut reader).unwrap();

        let response = ProduceResponse {
            topics: vec![ProduceTopicResponse {
                name: topic.to_string(),
                partitions: vec![ProducePartitionResponse {
                    partition_index: 0,
                    error_code,
                    base_offset: 0,
                    log_append_time: -1,
                }],
            }],
            throttle_time_ms: 0,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, &writer.into_vec())
            .await
            .unwrap();
        request
    }

    #[test]
    fn parse_headers_decodes_string_values_and_falls_back_to_json_for_others() {
        let headers = parse_headers(r#"{"source":"demo","count":3,"missing":null}"#);
        let as_map: std::collections::HashMap<_, _> = headers.into_iter().collect();
        assert_eq!(
            as_map.get("source").unwrap().as_deref(),
            Some(b"demo".as_slice())
        );
        assert_eq!(
            as_map.get("count").unwrap().as_deref(),
            Some(b"3".as_slice())
        );
        assert_eq!(
            as_map.get("missing").unwrap().as_deref(),
            Some(b"null".as_slice())
        );
    }

    #[test]
    fn parse_headers_returns_empty_for_malformed_or_non_object_json() {
        assert!(parse_headers("not json").is_empty());
        assert!(parse_headers("[1,2,3]").is_empty());
    }

    #[rusty_tokio::test]
    async fn relay_pending_publishes_and_marks_every_pending_entry_in_one_commit() {
        let mut conn = seeded_connection();
        let payload = json!({"a": 1});
        let headers = json!({"source": "demo"});
        write_outbox_entry(&conn, "A", "topic-a", &payload, Some(&headers)).unwrap();
        write_outbox_entry(&conn, "B", "topic-b", &payload, None).unwrap();

        let (client_io, mut peer) = duplex(8192);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            let first = respond_to_produce(&mut peer, "topic-a", 0).await;
            let second = respond_to_produce(&mut peer, "topic-b", 0).await;
            (first, second)
        });

        let published = relay_pending(&mut conn, &mut client, 100).await.unwrap();
        let (first, second) = server.await.unwrap();

        assert_eq!(published, 2);
        assert_eq!(first.topics[0].name, "topic-a");
        assert!(first.topics[0].partitions[0].records[0].key.is_none());
        assert_eq!(second.topics[0].name, "topic-b");

        let entries = fetch_all(&conn).unwrap();
        assert!(entries.iter().all(|e| e.published_at.is_some()));
    }

    #[rusty_tokio::test]
    async fn relay_pending_leaves_the_whole_batch_pending_when_one_produce_fails() {
        let mut conn = seeded_connection();
        let payload = json!({});
        write_outbox_entry(&conn, "A", "topic-a", &payload, None).unwrap();
        write_outbox_entry(&conn, "B", "topic-b", &payload, None).unwrap();

        let (client_io, mut peer) = duplex(8192);
        let mut client = KafkaClient::new(client_io, None);

        let server = rusty_tokio::spawn(async move {
            respond_to_produce(
                &mut peer, "topic-a", 3, /* UNKNOWN_TOPIC_OR_PARTITION */
            )
            .await;
        });

        let err = relay_pending(&mut conn, &mut client, 100)
            .await
            .unwrap_err();
        server.await.unwrap();

        assert!(matches!(err, RelayError::KafkaErrorCode(3)));
        let entries = fetch_all(&conn).unwrap();
        assert!(entries.iter().all(|e| e.published_at.is_none()));
    }

    #[rusty_tokio::test]
    async fn relay_pending_is_a_no_op_with_nothing_pending() {
        let mut conn = seeded_connection();
        let (client_io, _peer) = duplex(4096);
        let mut client = KafkaClient::new(client_io, None);

        let published = relay_pending(&mut conn, &mut client, 100).await.unwrap();
        assert_eq!(published, 0);
    }

    #[test]
    fn outbox_relay_stop_before_start_is_a_harmless_no_op() {
        let mut relay = OutboxRelay::new("unused.db", "unused:9092");
        relay.stop();
    }

    #[test]
    fn outbox_relay_start_then_stop_exits_promptly_with_no_broker_and_nothing_pending() {
        // No live broker in this environment (same limitation as every
        // other rusty_kafka-backed test in this crate family) -- this
        // exercises the thread-spawn/poll-loop/stop machinery itself,
        // not a real produce. Nothing pending means poll_once never
        // even tries to connect, so this doesn't depend on
        // "127.0.0.1:1" actually refusing quickly.
        let mut db_path = std::env::temp_dir();
        db_path.push(format!(
            "rusty_meshed_outbox_relay_test_{}.db",
            rusty_uuid::Uuid::new_v4()
        ));
        let db_path = db_path.to_str().unwrap().to_string();
        {
            let conn = Connection::open(&db_path).unwrap();
            ensure_schema(&conn).unwrap();
        }

        let mut relay = OutboxRelay::new(db_path.clone(), "127.0.0.1:1");
        relay.start();
        thread::sleep(Duration::from_millis(50));
        let started = std::time::Instant::now();
        relay.stop();
        assert!(started.elapsed() < Duration::from_secs(2));

        let _ = std::fs::remove_file(&db_path);
    }
}
