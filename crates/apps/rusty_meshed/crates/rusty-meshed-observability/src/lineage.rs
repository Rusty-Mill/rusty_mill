//! Data lineage tracking -- the Rust port of `meshed.observability.lineage`.
//!
//! - **Topology lineage**: which data products consume which other
//!   products, populated automatically from job-run records.
//! - **Record-level lineage**: the provenance chain for a given
//!   `correlation_id` across domain boundaries.
//!
//! Storage uses a fresh connection per call, matching the Python
//! source's per-call `sqlite3.connect()` (its own stated reason: no
//! thread-safety issues when multiple workers share one tracker
//! instance -- a `rusqlite::Connection` isn't `Sync`, so this crate
//! keeps the same per-call-connection shape rather than wrapping one
//! long-lived connection in a mutex).
//!
//! This port has no generic OpenLineage client to sit behind (there's
//! no Rust equivalent in this workspace, and this module is the only
//! thing in the source repo that ever constructs a `RunEvent`) --
//! [`LineageTracker::record_job_run`] writes the one event shape the
//! source ever actually produces (a `COMPLETE` run event) directly,
//! folding in `SQLiteLineageTransport.emit()`'s filtering/shaping logic
//! rather than modeling a separate transport layer that has nothing
//! else to filter. Two source details worth flagging since they're
//! easy to miss from the code alone:
//! - `job.namespace` is accepted as a parameter but never persisted --
//!   `SQLiteLineageTransport.emit()`'s insert only ever writes
//!   `job.name`. Preserved as-is.
//! - The source generates a `run_id` (`uuid.uuid4()`) for every
//!   `RunEvent` but the transport's insert never includes it -- it's
//!   generated and immediately discarded, with no observable effect.
//!   This port skips generating it at all, since there is nothing to
//!   preserve: no code path in the source ever reads it back.

use rusty_json::json;
use rusty_sqlite::rusqlite::{params, Connection};

/// One record-level lineage entry, as returned by
/// [`LineageTracker::get_record_lineage`].
#[derive(Debug, Clone, PartialEq)]
pub struct LineageRecord {
    pub event_id: String,
    pub correlation_id: String,
    pub source_event_ids: Vec<String>,
    pub product_name: String,
    pub topic_name: String,
    pub event_timestamp: String,
}

/// One topology dependency edge, as returned by
/// [`LineageTracker::get_topology_dependencies`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyDependency {
    pub consumer: String,
    pub input_topic: String,
}

/// High-level lineage recorder and query interface.
pub struct LineageTracker {
    db_path: String,
}

impl LineageTracker {
    /// Opens (creating if absent) the lineage tables at `db_path` and
    /// returns a tracker bound to that file. Table creation is
    /// idempotent (`CREATE TABLE IF NOT EXISTS`), matching every
    /// `LineageTracker()` construction in the source re-verifying the
    /// schema exists.
    pub fn new(db_path: impl Into<String>) -> rusty_sqlite::rusqlite::Result<Self> {
        let db_path = db_path.into();
        let conn = Connection::open(&db_path)?;
        ensure_schema(&conn)?;
        Ok(LineageTracker { db_path })
    }

    fn connect(&self) -> rusty_sqlite::rusqlite::Result<Connection> {
        Connection::open(&self.db_path)
    }

    /// Records a `COMPLETE` job-run event: one row in `lineage_events`
    /// per call. Each `(namespace, name)` pair in `inputs`/`outputs`
    /// becomes one dataset reference in the stored JSON arrays.
    /// `job_namespace` is accepted for interface parity but not
    /// persisted -- see the module doc.
    pub fn record_job_run(
        &self,
        job_name: &str,
        _job_namespace: &str,
        inputs: &[(String, String)],
        outputs: &[(String, String)],
    ) -> rusty_sqlite::rusqlite::Result<()> {
        let event_time = now_iso();
        let inputs_json = rusty_json::to_string(&datasets_json(inputs))
            .expect("serializing a datasets array never fails");
        let outputs_json = rusty_json::to_string(&datasets_json(outputs))
            .expect("serializing a datasets array never fails");

        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO lineage_events (job_name, event_type, event_time, inputs, outputs)
             VALUES (?1, 'COMPLETE', ?2, ?3, ?4)",
            params![job_name, event_time, inputs_json, outputs_json],
        )?;
        Ok(())
    }

    /// Returns distinct `(consumer, input_topic)` dependency pairs
    /// across every recorded `COMPLETE` job run, deduplicated so
    /// repeated runs of the same job don't duplicate the topology
    /// graph. Empty when no events have been recorded.
    pub fn get_topology_dependencies(
        &self,
    ) -> rusty_sqlite::rusqlite::Result<Vec<TopologyDependency>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT job_name, inputs FROM lineage_events WHERE event_type = 'COMPLETE'",
        )?;
        let rows = stmt.query_map([], |row| {
            let job_name: String = row.get(0)?;
            let inputs_json: String = row.get(1)?;
            Ok((job_name, inputs_json))
        })?;

        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for row in rows {
            let (job_name, inputs_json) = row?;
            let Ok(parsed) = rusty_json::from_str::<rusty_json::Value>(&inputs_json) else {
                continue;
            };
            let Some(datasets) = parsed.as_array() else {
                continue;
            };
            for dataset in datasets {
                let Some(name) = dataset.get("name").and_then(|v| v.as_str()) else {
                    continue;
                };
                let key = (job_name.clone(), name.to_string());
                if seen.insert(key) {
                    result.push(TopologyDependency {
                        consumer: job_name.clone(),
                        input_topic: name.to_string(),
                    });
                }
            }
        }
        Ok(result)
    }

    /// Persists a record-level lineage entry for a single domain event
    /// -- called by the SDK producer after each `publish()` so the
    /// event's lineage can be traced via its `correlation_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn record_event(
        &self,
        event_id: &str,
        correlation_id: &str,
        source_event_ids: &[String],
        product_name: &str,
        topic_name: &str,
        event_timestamp: &str,
    ) -> rusty_sqlite::rusqlite::Result<()> {
        let source_json =
            rusty_json::to_string(&source_event_ids.to_vec()).unwrap_or_else(|_| "[]".to_string());
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO lineage_records
                (event_id, correlation_id, source_event_ids, product_name, topic_name, event_timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![event_id, correlation_id, source_json, product_name, topic_name, event_timestamp],
        )?;
        Ok(())
    }

    /// Returns the provenance chain for `correlation_id`: every
    /// recorded event sharing it, ordered by `event_timestamp`
    /// ascending (a plain string sort, matching the source, not a
    /// parsed-datetime sort). Empty (not an error) for an unknown
    /// `correlation_id`.
    pub fn get_record_lineage(
        &self,
        correlation_id: &str,
    ) -> rusty_sqlite::rusqlite::Result<Vec<LineageRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT event_id, correlation_id, source_event_ids, product_name, topic_name, event_timestamp
             FROM lineage_records WHERE correlation_id = ?1 ORDER BY event_timestamp ASC",
        )?;
        let rows = stmt.query_map(params![correlation_id], |row| {
            let source_event_ids_json: String = row.get(2)?;
            let source_event_ids =
                rusty_json::from_str::<Vec<String>>(&source_event_ids_json).unwrap_or_default();
            Ok(LineageRecord {
                event_id: row.get(0)?,
                correlation_id: row.get(1)?,
                source_event_ids,
                product_name: row.get(3)?,
                topic_name: row.get(4)?,
                event_timestamp: row.get(5)?,
            })
        })?;
        rows.collect()
    }
}

fn ensure_schema(conn: &Connection) -> rusty_sqlite::rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS lineage_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_name TEXT NOT NULL,
            event_type TEXT NOT NULL,
            event_time TEXT NOT NULL,
            inputs TEXT NOT NULL,
            outputs TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS lineage_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            correlation_id TEXT NOT NULL,
            source_event_ids TEXT NOT NULL,
            product_name TEXT NOT NULL,
            topic_name TEXT NOT NULL,
            event_timestamp TEXT NOT NULL
        );",
    )
}

fn datasets_json(pairs: &[(String, String)]) -> rusty_json::Value {
    rusty_json::Value::Array(
        pairs
            .iter()
            .map(
                |(namespace, name)| json!({"namespace": namespace.as_str(), "name": name.as_str()}),
            )
            .collect(),
    )
}

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

    // Civil-from-days (Howard Hinnant's algorithm), proleptic Gregorian.
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

    /// `LineageTracker` opens a fresh connection per call (matching the
    /// Python source's per-call `sqlite3.connect()`) -- `:memory:` gives
    /// every connection its own isolated, empty database, which breaks
    /// that design entirely, so tests need a real (temporary) file
    /// instead. Wraps the tracker with a `Drop` that removes the file,
    /// and a unique name per call so parallel test execution doesn't
    /// collide.
    struct TempTracker {
        tracker: LineageTracker,
        path: std::path::PathBuf,
    }

    impl std::ops::Deref for TempTracker {
        type Target = LineageTracker;
        fn deref(&self) -> &LineageTracker {
            &self.tracker
        }
    }

    impl Drop for TempTracker {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn tracker() -> TempTracker {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusty_meshed_lineage_test_{}_{n}.db",
            std::process::id()
        ));
        let tracker = LineageTracker::new(path.to_str().unwrap()).unwrap();
        TempTracker { tracker, path }
    }

    #[test]
    fn record_job_run_and_get_topology_dependencies_round_trip() {
        let tracker = tracker();
        tracker
            .record_job_run(
                "readiness-reporting",
                "meshed",
                &[(
                    "kafka".to_string(),
                    "manpower.personnel-lifecycle.assignments".to_string(),
                )],
                &[],
            )
            .unwrap();

        let deps = tracker.get_topology_dependencies().unwrap();
        assert_eq!(
            deps,
            vec![TopologyDependency {
                consumer: "readiness-reporting".to_string(),
                input_topic: "manpower.personnel-lifecycle.assignments".to_string(),
            }]
        );
    }

    #[test]
    fn get_topology_dependencies_dedupes_repeated_job_runs() {
        let tracker = tracker();
        for _ in 0..3 {
            tracker
                .record_job_run(
                    "readiness-reporting",
                    "meshed",
                    &[(
                        "kafka".to_string(),
                        "manpower.personnel-lifecycle.assignments".to_string(),
                    )],
                    &[],
                )
                .unwrap();
        }
        let deps = tracker.get_topology_dependencies().unwrap();
        assert_eq!(deps.len(), 1);
    }

    #[test]
    fn get_topology_dependencies_is_empty_with_no_events() {
        let tracker = tracker();
        assert!(tracker.get_topology_dependencies().unwrap().is_empty());
    }

    #[test]
    fn record_event_and_get_record_lineage_round_trip() {
        let tracker = tracker();
        tracker
            .record_event(
                "event-1",
                "corr-1",
                &["event-0".to_string()],
                "personnel-lifecycle",
                "manpower.personnel-lifecycle.assignments",
                "2026-01-01T00:00:00Z",
            )
            .unwrap();

        let records = tracker.get_record_lineage("corr-1").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event_id, "event-1");
        assert_eq!(records[0].source_event_ids, vec!["event-0".to_string()]);
        assert_eq!(records[0].product_name, "personnel-lifecycle");
    }

    #[test]
    fn get_record_lineage_orders_by_timestamp_ascending() {
        let tracker = tracker();
        tracker
            .record_event("e2", "corr-1", &[], "p", "t", "2026-01-02T00:00:00Z")
            .unwrap();
        tracker
            .record_event("e1", "corr-1", &[], "p", "t", "2026-01-01T00:00:00Z")
            .unwrap();

        let records = tracker.get_record_lineage("corr-1").unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].event_id, "e1");
        assert_eq!(records[1].event_id, "e2");
    }

    #[test]
    fn get_record_lineage_is_empty_for_unknown_correlation_id_not_an_error() {
        let tracker = tracker();
        let records = tracker
            .get_record_lineage("no-such-correlation-id")
            .unwrap();
        assert!(records.is_empty());
    }
}
