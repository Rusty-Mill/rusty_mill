//! The transactional outbox pattern -- the Rust port of
//! `meshed.infrastructure.outbox` (SDK-055..064).
//!
//! [`OutboxEntry`] (SDK-055) and [`write_outbox_entry`] (SDK-056) are
//! fully ported: plain data plus a SQLite insert that deliberately
//! doesn't manage its own transaction boundary, so a caller can wrap
//! it and their own business-entity write in one atomic
//! `rusqlite::Transaction` -- the core invariant this pattern exists
//! for (see [`write_outbox_entry`]'s own doc for how).
//!
//! `OutboxRelay` (SDK-057..064) is **not** ported here: it publishes
//! pending entries to Kafka, which needs a `Produce` request
//! `rusty_kafka` doesn't implement (the same gap flagged on
//! `rusty-meshed-observability::slo`'s `SLOViolationPublisher` and
//! GitHub issue #87). A relay that can never publish isn't a
//! meaningful partial capability, so this is deferred as one cluster
//! rather than built halfway.

use rusty_sqlite::rusqlite::{params, Connection, Result as SqlResult};

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
    /// `None` while pending; set by the (not yet implemented) relay
    /// after a successful publish.
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
}
