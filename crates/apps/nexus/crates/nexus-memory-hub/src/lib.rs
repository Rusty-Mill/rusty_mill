//! Central sync hub for Nexus memory.
//!
//! A standalone HTTP server that many Nexus instances push their memories to
//! and pull each other's from — the durable convergence point behind
//! "memory shared across Nexus implementations". It mirrors the proven
//! `remind_me` hub wire protocol:
//!
//! - `GET  /health`      — unauthenticated liveness probe.
//! - `POST /sync/push`   — bearer-authed batch upsert; last-write-wins on
//!   `updated_at`; replies `{ accepted, processed_ids, failed }`.
//! - `GET  /sync/pull`   — bearer-authed keyset page of records newer than a
//!   `(since, since_id)` cursor, optionally excluding one node; replies
//!   `{ records, count }`.
//!
//! The hub is deliberately **schema-agnostic**: each record is stored by its
//! `id` with its `updated_at` (the LWW key + cursor), the authoring `node_id`,
//! the pushing `origin_node` (hub-only bookkeeping, never returned), and the
//! whole record as an opaque JSON `payload`. New memory fields therefore need
//! no hub change. Conflict resolution is last-write-wins on the canonical
//! ISO-8601-UTC `updated_at` string (lexically orderable). Auth is a single
//! shared `SYNC_SECRET` bearer token, constant-time compared; there is no node
//! registry (any `node_id` is accepted), matching `remind_me`.

#![warn(clippy::pedantic)]

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Epoch default for `since` when a client omits it (pull everything).
const EPOCH: &str = "1970-01-01T00:00:00+00:00";
/// Hard cap on a single pull page (matches `remind_me`).
const MAX_PULL_LIMIT: usize = 500;
/// Default pull page size when the client omits `limit`.
const DEFAULT_PULL_LIMIT: usize = 100;
/// Clock-skew tolerance for a pushed `updated_at`: a timestamp further ahead
/// of the hub's own clock than this is rejected rather than persisted. An
/// unbounded future `updated_at` would permanently outrank every future
/// genuine update in the last-write-wins comparison, freezing the record.
const MAX_FUTURE_SKEW_MINUTES: i64 = 5;

/// Errors from the hub store layer.
#[derive(Debug, thiserror::Error)]
pub enum HubError {
    /// Underlying `SQLite` error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A pull cursor that is not an RFC 3339 timestamp.
    #[error("bad cursor: {0}")]
    BadCursor(String),
    /// Connection-pool error.
    #[error("pool: {0}")]
    Pool(#[from] r2d2::Error),
}

/// Result alias for the hub store.
pub type Result<T> = std::result::Result<T, HubError>;

/// Schema applied on open. One generic, schema-agnostic table.
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS records (
    id          TEXT PRIMARY KEY,
    updated_at  TEXT NOT NULL,
    node_id     TEXT,
    origin_node TEXT,
    payload     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_records_cursor ON records(updated_at, id);
CREATE TABLE IF NOT EXISTS records_rejected (
    id          TEXT,
    updated_at  TEXT,
    payload     TEXT NOT NULL,
    reason      TEXT NOT NULL,
    moved_at    TEXT NOT NULL
);";

/// `PRAGMA user_version` once every stored `updated_at` is canonical.
const CANONICAL_KEYS_VERSION: i64 = 1;

/// Durable convergence store backing the hub.
#[derive(Clone)]
pub struct HubStore {
    pool: Pool<SqliteConnectionManager>,
}

impl std::fmt::Debug for HubStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubStore").finish_non_exhaustive()
    }
}

fn init_conn(conn: &mut rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;\nPRAGMA busy_timeout=5000;")
}

/// The ordering key for an RFC 3339 `updated_at`: UTC, fixed nanosecond
/// precision, `Z` — so string order is time order, whatever offset or
/// precision the pushing node wrote (design review 2.8 / N3). `None` when
/// `updated_at` is not RFC 3339. The payload keeps the node's own string;
/// only the stored key and the pull cursor use this form.
fn canonical_ts(updated_at: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(updated_at).ok().map(|t| {
        t.with_timezone(&Utc)
            .format("%Y-%m-%dT%H:%M:%S%.9fZ")
            .to_string()
    })
}

/// True if a canonical key is further in the future than
/// [`MAX_FUTURE_SKEW_MINUTES`] beyond the hub's own clock.
fn is_implausibly_future(canonical: &str) -> bool {
    let limit = Utc::now() + Duration::minutes(MAX_FUTURE_SKEW_MINUTES);
    canonical_ts(&limit.to_rfc3339()).is_some_and(|limit| canonical > limit.as_str())
}

/// Rewrite every stored `updated_at` into [`canonical_ts`] form, once
/// (gated on `PRAGMA user_version`). A row whose key never parsed — which
/// an older hub accepted and which then outranked every real timestamp —
/// moves to `records_rejected` with its payload, instead of being deleted.
fn canonicalize_stored_keys(conn: &mut rusqlite::Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= CANONICAL_KEYS_VERSION {
        return Ok(());
    }
    let tx = conn.transaction()?;
    let rows: Vec<(String, String)> = {
        let mut stmt = tx.prepare("SELECT id, updated_at FROM records")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    let now = Utc::now().to_rfc3339();
    for (id, updated_at) in rows {
        match canonical_ts(&updated_at) {
            Some(key) if key == updated_at => {}
            Some(key) => {
                tx.execute(
                    "UPDATE records SET updated_at = ?2 WHERE id = ?1",
                    params![id, key],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO records_rejected (id, updated_at, payload, reason, moved_at) \
                     SELECT id, updated_at, payload, 'updated_at is not RFC 3339', ?2 \
                     FROM records WHERE id = ?1",
                    params![id, now],
                )?;
                tx.execute("DELETE FROM records WHERE id = ?1", params![id])?;
            }
        }
    }
    tx.pragma_update(None, "user_version", CANONICAL_KEYS_VERSION)?;
    tx.commit()?;
    Ok(())
}

/// What [`HubStore::push`] did with a batch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PushOutcome {
    /// Ids accepted — valid and handled.
    pub processed: Vec<String>,
    /// Records refused, each with why.
    pub rejected: Vec<Rejected>,
}

/// A pushed record the hub refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rejected {
    /// Its `id`, when it has one.
    pub id: Option<String>,
    /// Why it was refused.
    pub reason: String,
}

impl HubStore {
    /// Open (creating if needed) a hub database at `path`.
    ///
    /// # Errors
    /// Returns an error if the pool can't be built or the schema can't apply.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(init_conn);
        let store = Self {
            pool: Pool::new(manager)?,
        };
        let mut conn = store.pool.get()?;
        conn.execute_batch(SCHEMA)?;
        canonicalize_stored_keys(&mut conn)?;
        drop(conn);
        Ok(store)
    }

    /// Open an in-memory hub backed by a single shared connection (tests).
    ///
    /// # Errors
    /// Returns an error if the pool can't be built or the schema can't apply.
    pub fn open_in_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory().with_init(init_conn);
        let store = Self {
            pool: Pool::builder().max_size(1).build(manager)?,
        };
        let mut conn = store.pool.get()?;
        conn.execute_batch(SCHEMA)?;
        canonicalize_stored_keys(&mut conn)?;
        drop(conn);
        Ok(store)
    }

    /// Upsert a batch of records, last-write-wins on `updated_at` compared as
    /// time (its [`canonical_ts`] key), not as text. `origin_node` is the
    /// pushing node (recorded for `exclude_node` filtering, never returned on
    /// pull). A record lacking a string `id` or `updated_at`, with an
    /// `updated_at` that is not RFC 3339, or one more than
    /// [`MAX_FUTURE_SKEW_MINUTES`] ahead of the hub's clock, is refused with
    /// its reason rather than persisted.
    ///
    /// # Errors
    /// Returns an error on a write failure.
    pub fn push(&self, origin_node: &str, records: &[Value]) -> Result<PushOutcome> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let mut outcome = PushOutcome::default();
        {
            let mut stmt = tx.prepare(
                "INSERT INTO records (id, updated_at, node_id, origin_node, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                     updated_at = excluded.updated_at,
                     node_id = excluded.node_id,
                     origin_node = excluded.origin_node,
                     payload = excluded.payload
                 WHERE excluded.updated_at > records.updated_at;",
            )?;
            for record in records {
                let id = record.get("id").and_then(Value::as_str);
                let refuse = |reason: &str| Rejected {
                    id: id.map(str::to_string),
                    reason: reason.to_string(),
                };
                let (Some(id), Some(updated_at)) =
                    (id, record.get("updated_at").and_then(Value::as_str))
                else {
                    outcome
                        .rejected
                        .push(refuse("missing string id or updated_at"));
                    continue;
                };
                let Some(key) = canonical_ts(updated_at) else {
                    // Compared as text it would outrank every real timestamp.
                    outcome.rejected.push(refuse("updated_at is not RFC 3339"));
                    continue;
                };
                if is_implausibly_future(&key) {
                    // Would permanently win every future LWW comparison.
                    outcome
                        .rejected
                        .push(refuse("updated_at is too far in the future"));
                    continue;
                }
                let node_id = record.get("node_id").and_then(Value::as_str);
                let payload = record.to_string();
                stmt.execute(params![id, key, node_id, origin_node, payload])?;
                outcome.processed.push(id.to_string());
            }
        }
        tx.commit()?;
        Ok(outcome)
    }

    /// Pull a keyset page of records strictly after the `(since, since_id)`
    /// cursor, newest-cursor-last, optionally excluding records pushed by
    /// `exclude_node`. When `since_id` is `None`, a strict `updated_at > since`
    /// is used (first-page / legacy). `since` is any RFC 3339 form (a client
    /// echoes a payload's own string) and is compared as time. Returns the
    /// opaque record payloads.
    ///
    /// # Errors
    /// [`HubError::BadCursor`] when `since` is not RFC 3339; otherwise a query
    /// or decode failure.
    pub fn pull(
        &self,
        since: &str,
        since_id: Option<&str>,
        exclude_node: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>> {
        use std::fmt::Write as _;

        let limit = limit.clamp(1, MAX_PULL_LIMIT);
        let since = canonical_ts(since).ok_or_else(|| HubError::BadCursor(since.to_string()))?;
        let conn = self.pool.get()?;

        // Build the WHERE incrementally with positional params.
        let mut sql = String::from("SELECT payload FROM records WHERE ");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(sid) = since_id {
            sql.push_str("(updated_at > ?1 OR (updated_at = ?1 AND id > ?2))");
            args.push(Box::new(since.clone()));
            args.push(Box::new(sid.to_string()));
        } else {
            sql.push_str("updated_at > ?1");
            args.push(Box::new(since));
        }
        if let Some(node) = exclude_node {
            let _ = write!(
                sql,
                " AND (origin_node IS NULL OR origin_node != ?{})",
                args.len() + 1
            );
            args.push(Box::new(node.to_string()));
        }
        let _ = write!(sql, " ORDER BY updated_at ASC, id ASC LIMIT {limit}");

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(AsRef::as_ref).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let payload: String = row.get(0)?;
                Ok(payload)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|p| serde_json::from_str::<Value>(&p).ok())
            .collect())
    }

    /// Total stored records.
    ///
    /// # Errors
    /// Returns an error on a query failure.
    pub fn count(&self) -> Result<u64> {
        let conn = self.pool.get()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM records", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

// ── HTTP wire types ────────────────────────────────────────────────────────

/// Body of `POST /sync/push`.
#[derive(Debug, Deserialize)]
pub struct PushRequest {
    /// The pushing node's id (recorded as `origin_node`).
    #[serde(default)]
    pub node_id: String,
    /// Records to upsert (opaque JSON objects carrying at least `id` +
    /// `updated_at`).
    #[serde(default)]
    pub records: Vec<Value>,
}

/// Reply for `POST /sync/push`.
#[derive(Debug, Serialize)]
pub struct PushResponse {
    /// Number of records accepted (valid and handled).
    pub accepted: usize,
    /// Ids accepted — the client marks exactly these sent.
    pub processed_ids: Vec<String>,
    /// Number of records refused.
    pub failed: usize,
    /// Each refused record's id (if any) and reason.
    pub rejected: Vec<Rejected>,
}

/// Query params for `GET /sync/pull`.
#[derive(Debug, Deserialize)]
pub struct PullQuery {
    /// Keyset cursor timestamp (default epoch).
    pub since: Option<String>,
    /// Keyset cursor id (for boundary ties).
    pub since_id: Option<String>,
    /// Node whose own pushes should be excluded.
    pub exclude_node: Option<String>,
    /// Max records to return (clamped to 1..=500).
    pub limit: Option<usize>,
}

/// Reply for `GET /sync/pull`.
#[derive(Debug, Serialize)]
pub struct PullResponse {
    /// The record payloads, cursor order.
    pub records: Vec<Value>,
    /// `records.len()`, for convenience.
    pub count: usize,
}

/// Shared server state.
#[derive(Clone)]
pub struct AppState {
    /// The convergence store.
    pub store: HubStore,
    /// Shared bearer secret all clients must present.
    pub secret: Arc<String>,
}

/// Build the hub's axum router over `state`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/sync/push", post(push))
        .route("/sync/pull", get(pull))
        .with_state(state)
}

/// Serve the hub over `listener` until the task is dropped. A thin wrapper over
/// [`router`] + `axum::serve` so embedders and tests need not depend on axum.
///
/// # Errors
/// Propagates the server's I/O error, if any.
pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

/// Constant-time byte-equality so token checks don't leak length-prefix timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Verify the `Authorization: Bearer <secret>` header.
fn authorize(headers: &HeaderMap, secret: &str) -> std::result::Result<(), (StatusCode, String)> {
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(token) if ct_eq(token.as_bytes(), secret.as_bytes()) => Ok(()),
        _ => Err((StatusCode::UNAUTHORIZED, "unauthorized".to_string())),
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    role: &'static str,
    records: u64,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        role: "hub",
        records: state.store.count().unwrap_or(0),
    })
}

async fn push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PushRequest>,
) -> std::result::Result<Json<PushResponse>, (StatusCode, String)> {
    authorize(&headers, &state.secret)?;
    let outcome = state
        .store
        .push(&req.node_id, &req.records)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("push: {e}")))?;
    Ok(Json(PushResponse {
        accepted: outcome.processed.len(),
        failed: outcome.rejected.len(),
        processed_ids: outcome.processed,
        rejected: outcome.rejected,
    }))
}

async fn pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PullQuery>,
) -> std::result::Result<Json<PullResponse>, (StatusCode, String)> {
    authorize(&headers, &state.secret)?;
    let since = q.since.unwrap_or_else(|| EPOCH.to_string());
    let records = state
        .store
        .pull(
            &since,
            q.since_id.as_deref(),
            q.exclude_node.as_deref(),
            q.limit.unwrap_or(DEFAULT_PULL_LIMIT),
        )
        .map_err(|e| match e {
            HubError::BadCursor(_) => (StatusCode::BAD_REQUEST, format!("pull: {e}")),
            other => (StatusCode::INTERNAL_SERVER_ERROR, format!("pull: {other}")),
        })?;
    let count = records.len();
    Ok(Json(PullResponse { records, count }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(id: &str, updated_at: &str, node: &str) -> Value {
        json!({ "id": id, "updated_at": updated_at, "node_id": node, "content": format!("c-{id}") })
    }

    #[test]
    fn push_then_pull_round_trips() {
        let store = HubStore::open_in_memory().unwrap();
        let processed = store
            .push(
                "node-a",
                &[rec("m1", "2026-01-01T00:00:00+00:00", "node-a")],
            )
            .unwrap()
            .processed;
        assert_eq!(processed, vec!["m1"]);
        let out = store.pull(EPOCH, None, None, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["content"], "c-m1");
    }

    #[test]
    fn push_skips_records_without_id_or_updated_at() {
        let store = HubStore::open_in_memory().unwrap();
        let processed = store
            .push(
                "node-a",
                &[
                    rec("m1", "2026-01-01T00:00:00+00:00", "node-a"),
                    json!({ "content": "no id/ts" }),
                ],
            )
            .unwrap()
            .processed;
        assert_eq!(processed, vec!["m1"]); // the invalid one is skipped
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn last_write_wins_on_updated_at() {
        let store = HubStore::open_in_memory().unwrap();
        store
            .push("a", &[rec("m1", "2026-01-01T00:00:00+00:00", "a")])
            .unwrap();
        // Older update is ignored.
        store
            .push(
                "b",
                &[json!({ "id": "m1", "updated_at": "2025-06-01T00:00:00+00:00", "content": "stale" })],
            )
            .unwrap();
        let out = store.pull(EPOCH, None, None, 100).unwrap();
        assert_eq!(out[0]["content"], "c-m1");
        // Newer update wins.
        store
            .push(
                "b",
                &[json!({ "id": "m1", "updated_at": "2026-06-01T00:00:00+00:00", "content": "fresh" })],
            )
            .unwrap();
        let out = store.pull(EPOCH, None, None, 100).unwrap();
        assert_eq!(out[0]["content"], "fresh");
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn pull_keyset_cursor_paginates_without_skipping_ties() {
        let store = HubStore::open_in_memory().unwrap();
        // Three rows share a timestamp — the keyset must page by id, not skip.
        let ts = "2026-01-01T00:00:00+00:00";
        store
            .push(
                "a",
                &[rec("m1", ts, "a"), rec("m2", ts, "a"), rec("m3", ts, "a")],
            )
            .unwrap();
        let page1 = store.pull(EPOCH, None, None, 2).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0]["id"], "m1");
        assert_eq!(page1[1]["id"], "m2");
        // Resume from the last seen (ts, "m2").
        let page2 = store.pull(ts, Some("m2"), None, 2).unwrap();
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0]["id"], "m3");
    }

    #[test]
    fn pull_excludes_origin_node() {
        let store = HubStore::open_in_memory().unwrap();
        store
            .push("a", &[rec("m1", "2026-01-01T00:00:00+00:00", "a")])
            .unwrap();
        store
            .push("b", &[rec("m2", "2026-01-02T00:00:00+00:00", "b")])
            .unwrap();
        // Node "a" pulls everything except what it pushed.
        let out = store.pull(EPOCH, None, Some("a"), 100).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], "m2");
    }

    #[test]
    fn ct_eq_matches_only_identical() {
        assert!(ct_eq(b"secret", b"secret"));
        assert!(!ct_eq(b"secret", b"secrEt"));
        assert!(!ct_eq(b"secret", b"secret-longer"));
    }

    #[test]
    fn is_implausibly_future_flags_far_future_only() {
        let key = |s: &str| canonical_ts(s).unwrap();
        assert!(!is_implausibly_future(&key("2020-01-01T00:00:00+00:00")));
        assert!(is_implausibly_future(&key("9999-01-01T00:00:00+00:00")));
    }

    #[test]
    fn push_rejects_implausible_future_timestamp() {
        let store = HubStore::open_in_memory().unwrap();
        // A node pushing a year-9999 `updated_at` must not be able to
        // permanently freeze the record against every future genuine update.
        let processed = store
            .push(
                "attacker",
                &[rec("m1", "9999-01-01T00:00:00+00:00", "attacker")],
            )
            .unwrap()
            .processed;
        assert!(
            processed.is_empty(),
            "far-future timestamp must be rejected"
        );
        assert_eq!(store.count().unwrap(), 0);

        // A legitimate, plausible update still writes normally afterward.
        let processed = store
            .push(
                "node-a",
                &[rec("m1", "2026-06-01T00:00:00+00:00", "node-a")],
            )
            .unwrap()
            .processed;
        assert_eq!(processed, vec!["m1"]);
        let out = store.pull(EPOCH, None, None, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["content"], "c-m1");
    }

    /// Review 2.8 (N3): a malformed `updated_at` such as `"zzzz"` is refused
    /// with a reason; compared as text it would have outranked every real
    /// timestamp and frozen the record.
    #[test]
    fn a_malformed_updated_at_is_refused_with_a_reason() {
        let store = HubStore::open_in_memory().unwrap();
        let out = store.push("peer", &[rec("m1", "zzzz", "peer")]).unwrap();
        assert!(out.processed.is_empty());
        assert_eq!(
            out.rejected,
            vec![Rejected {
                id: Some("m1".into()),
                reason: "updated_at is not RFC 3339".into()
            }]
        );
        store
            .push("a", &[rec("m1", "2026-01-01T00:00:00Z", "a")])
            .unwrap();
        assert_eq!(
            store.pull(EPOCH, None, None, 10).unwrap()[0]["content"],
            "c-m1"
        );
    }

    /// Review 2.8: LWW compares instants, not strings — offsets and
    /// fractional precision do not change which write is newer.
    #[test]
    fn last_write_wins_compares_instants_across_offsets_and_precision() {
        let store = HubStore::open_in_memory().unwrap();
        // 10:00:00.5 UTC, written with a +02:00 offset.
        store
            .push("a", &[json!({"id": "m1", "updated_at": "2026-01-01T12:00:00.5+02:00", "content": "later"})])
            .unwrap();
        // 10:00:00 UTC exactly: earlier, although it sorts after as text.
        store
            .push(
                "b",
                &[json!({"id": "m1", "updated_at": "2026-01-01T10:00:00Z", "content": "earlier"})],
            )
            .unwrap();
        assert_eq!(
            store.pull(EPOCH, None, None, 10).unwrap()[0]["content"],
            "later"
        );
        // The same instant in another form is not newer.
        store
            .push("b", &[json!({"id": "m1", "updated_at": "2026-01-01T10:00:00.500000000+00:00", "content": "same"})])
            .unwrap();
        assert_eq!(
            store.pull(EPOCH, None, None, 10).unwrap()[0]["content"],
            "later"
        );
        // A cursor in any offset resumes at the right instant.
        let after = store
            .pull("2026-01-01T11:00:00+02:00", None, None, 10)
            .unwrap();
        assert_eq!(after.len(), 1, "09:00Z < 10:00:00.5Z");
    }

    /// Review 2.8: a pull cursor that is not RFC 3339 is refused.
    #[test]
    fn a_malformed_pull_cursor_is_refused() {
        let store = HubStore::open_in_memory().unwrap();
        assert!(matches!(
            store.pull("zzzz", None, None, 10),
            Err(HubError::BadCursor(_))
        ));
    }

    /// Review 2.8: a hub written before canonical keys reopens with its
    /// keys canonicalized, and a row whose key never parsed moves to
    /// `records_rejected` instead of outranking every real write.
    #[test]
    fn an_older_hub_is_canonicalized_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            for (id, ts) in [("ok", "2026-01-01T12:00:00+02:00"), ("poison", "zzzz")] {
                conn.execute(
                    "INSERT INTO records (id, updated_at, payload) VALUES (?1, ?2, ?3)",
                    params![id, ts, rec(id, ts, "old").to_string()],
                )
                .unwrap();
            }
        }
        let store = HubStore::open(&path).unwrap();
        let conn = store.pool.get().unwrap();
        let key: String = conn
            .query_row("SELECT updated_at FROM records WHERE id = 'ok'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(key, "2026-01-01T10:00:00.000000000Z");
        let moved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM records_rejected WHERE id = 'poison'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!((store.count().unwrap(), moved), (1, 1));
        drop(conn);
        store
            .push("a", &[rec("poison", "2026-02-01T00:00:00Z", "a")])
            .unwrap();
        let out = store.pull(EPOCH, None, None, 10).unwrap();
        assert_eq!(out.len(), 2, "the poisoned id takes genuine writes again");
    }
}
