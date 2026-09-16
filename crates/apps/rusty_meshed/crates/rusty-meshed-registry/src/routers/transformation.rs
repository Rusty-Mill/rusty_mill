//! Digital transformation simulator endpoints -- the Rust port of
//! `meshed.transformation.router` (XFM-027..035). Three regular
//! request/response endpoints wrapping the already-built engine
//! (`crate::transformation`), plus one SSE stream.
//!
//! Like `routers::lineage`, the SSE endpoint doesn't go through
//! [`crate::app::AppState`]: it opens its own connection against a
//! hardcoded `"meshed_registry.db"` path, matching the source's
//! `_event_generator(_DEFAULT_DB_PATH)` -- a separate raw
//! `sqlite3.connect()` outside the request-scoped `SessionDep` the
//! other three endpoints use. Preserved deliberately, same reasoning
//! as `routers::lineage`'s module doc.

use super::detail_error;
use crate::app::AppState;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use crate::transformation::{
    advance_quarter, get_state, queue_decision, seed_transformation_state, DecisionRef,
    DecisionType, LegacySystem, MaturityPoint, TransformationState,
};
use rusty_http::StatusCode;
use rusty_request::Json;
use rusty_sqlite::rusqlite::{params, Connection};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

/// The source's own SSE module constant, same default as
/// `routers::lineage::DEFAULT_DB_PATH`.
pub const DEFAULT_DB_PATH: &str = "meshed_registry.db";

fn internal_error() -> Response {
    detail_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

fn session_error() -> Response {
    detail_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Database engine is not initialized",
    )
}

fn legacy_system_json(system: &LegacySystem) -> Json {
    let mut json = Json::object();
    json.insert("track", system.track.as_str());
    json.insert("name", system.name.as_str());
    json.insert("target_data_product", system.target_data_product.as_str());
    json.insert("status", system.status.as_str());
    json.insert("status_since_quarter", system.status_since_quarter);
    json
}

fn maturity_point_json(point: &MaturityPoint) -> Json {
    let mut json = Json::object();
    json.insert("quarter", point.quarter);
    json.insert("maturity_index", point.maturity_index);
    json
}

fn decision_ref_json(decision: &DecisionRef) -> Json {
    let mut json = Json::object();
    json.insert("id", decision.id);
    json.insert("quarter", decision.quarter);
    json.insert("decision_type", decision.decision_type.as_str());
    json.insert("target", decision.target.as_str());
    json
}

/// Matches `get_state()`'s dict shape exactly (REG/XFM's own
/// docs already cite this as the reference shape).
fn state_json(state: &TransformationState) -> Json {
    let mut legacy_systems = Json::array();
    for system in &state.legacy_systems {
        legacy_systems.push(legacy_system_json(system));
    }

    let mut capability = Json::object();
    for (track, dimensions) in &state.capability {
        let mut dimension_scores = Json::object();
        for (dimension, score) in dimensions {
            dimension_scores.insert(dimension.as_str(), *score);
        }
        capability.insert(track.as_str(), dimension_scores);
    }

    let mut maturity_trend = Json::array();
    for point in &state.maturity_trend {
        maturity_trend.push(maturity_point_json(point));
    }

    let mut pending_decisions = Json::array();
    for decision in &state.pending_decisions {
        pending_decisions.push(decision_ref_json(decision));
    }
    let mut decision_history = Json::array();
    for decision in &state.decision_history {
        decision_history.push(decision_ref_json(decision));
    }

    let mut json = Json::object();
    json.insert("quarter", state.quarter);
    json.insert("legacy_systems", legacy_systems);
    json.insert("capability", capability);
    json.insert("maturity_trend", maturity_trend);
    json.insert("pending_decisions", pending_decisions);
    json.insert("decision_history", decision_history);
    json
}

async fn state(app_state: Arc<AppState>, _req: Request) -> Response {
    let Ok(conn) = app_state.get_session() else {
        return session_error();
    };
    if seed_transformation_state(&conn).is_err() {
        return internal_error();
    }
    match get_state(&conn) {
        Ok(snapshot) => Response::json(StatusCode::OK, &state_json(&snapshot)),
        Err(_) => internal_error(),
    }
}

async fn create_decision(app_state: Arc<AppState>, req: Request) -> Response {
    let Ok(conn) = app_state.get_session() else {
        return session_error();
    };
    if seed_transformation_state(&conn).is_err() {
        return internal_error();
    }

    let Ok(body) = req.json() else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid JSON body");
    };
    let decision_type_raw = body.get("decision_type").and_then(|v| v.as_str());
    let target = body.get("target").and_then(|v| v.as_str());
    let (Some(decision_type_raw), Some(target)) = (decision_type_raw, target) else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "decision_type and target are both required",
        );
    };
    let Some(decision_type) = DecisionType::parse(decision_type_raw) else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("'{decision_type_raw}' is not a valid decision_type"),
        );
    };

    if decision_type.targets_a_track() {
        let track_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM legacy_systems WHERE track = ?1)",
                params![target],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !track_exists {
            return detail_error(StatusCode::NOT_FOUND, format!("Unknown track '{target}'."));
        }
    } else if target != "platform" && target != "product_teams" {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{} target must be one of {{'platform', 'product_teams'}}.",
                decision_type.as_str()
            ),
        );
    }

    match queue_decision(&conn, decision_type, target) {
        Ok(decision) => Response::json(StatusCode::CREATED, &decision_ref_json(&decision)),
        Err(_) => internal_error(),
    }
}

async fn advance(app_state: Arc<AppState>, _req: Request) -> Response {
    let Ok(mut conn) = app_state.get_session() else {
        return session_error();
    };
    if seed_transformation_state(&conn).is_err() {
        return internal_error();
    }
    match advance_quarter(&mut conn) {
        Ok(snapshot) => Response::json(StatusCode::OK, &state_json(&snapshot)),
        Err(_) => internal_error(),
    }
}

// ---------------------------------------------------------------------
// SSE event stream (XFM-030..034)
// ---------------------------------------------------------------------

/// One `event_id`-ordered poll cycle's worth of state, carried between
/// [`TransformationEventStream::next_chunk`] calls -- the Rust
/// equivalent of the source's `_event_generator`'s closure-captured
/// `last_id`. `poll_interval` is a test seam (production always uses
/// XFM-032's literal 1.0s; a test passes a near-zero duration so the
/// suite doesn't spend real wall-clock time on it).
struct TransformationEventStream {
    db_path: String,
    poll_interval: Duration,
    last_id: i64,
    initialized: bool,
    pending: VecDeque<String>,
}

impl TransformationEventStream {
    fn new(db_path: String) -> Self {
        TransformationEventStream::with_poll_interval(db_path, Duration::from_secs(1))
    }

    fn with_poll_interval(db_path: String, poll_interval: Duration) -> Self {
        TransformationEventStream {
            db_path,
            poll_interval,
            last_id: 0,
            initialized: false,
            pending: VecDeque::new(),
        }
    }

    /// XFM-031: on first call only, seeks `last_id = MAX(id)` so only
    /// events newer than connect-time stream -- defaults to 0 (full
    /// history) on any error, including a missing table/db.
    fn seed_last_id(&mut self) {
        self.initialized = true;
        let Ok(conn) = Connection::open(&self.db_path) else {
            return;
        };
        if let Ok(max_id) = conn.query_row("SELECT MAX(id) FROM transformation_events", [], |row| {
            row.get::<_, Option<i64>>(0)
        }) {
            self.last_id = max_id.unwrap_or(0);
        }
    }

    /// XFM-032..034: polls for up to 50 new rows past `last_id`;
    /// swallows all sqlite errors silently (an empty poll, not a
    /// stream failure) rather than propagating them.
    fn poll(&mut self) {
        let Ok(conn) = Connection::open(&self.db_path) else {
            return;
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, quarter, event_type, track, message, timestamp \
             FROM transformation_events WHERE id > ?1 ORDER BY id ASC LIMIT 50",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map(params![self.last_id], |row| {
            let id: i64 = row.get(0)?;
            let quarter: i64 = row.get(1)?;
            let event_type: String = row.get(2)?;
            let track: Option<String> = row.get(3)?;
            let message: String = row.get(4)?;
            let timestamp: String = row.get(5)?;
            Ok((id, quarter, event_type, track, message, timestamp))
        }) else {
            return;
        };

        for row in rows.flatten() {
            let (id, quarter, event_type, track, message, timestamp) = row;
            self.last_id = id;
            let mut json = Json::object();
            json.insert("id", id);
            json.insert("quarter", quarter);
            json.insert("eventType", event_type.as_str());
            json.insert("track", track);
            json.insert("message", message.as_str());
            json.insert("timestamp", timestamp.as_str());
            self.pending
                .push_back(format!("data: {}\n\n", json.to_json_string()));
        }
    }

    /// Produces the next SSE chunk, polling (and sleeping
    /// `poll_interval` once the current batch is drained) exactly as
    /// the source's `while True` generator body does: query, yield
    /// every matched row (or one heartbeat if none), then sleep once
    /// per cycle regardless of which happened.
    async fn next_chunk(&mut self) -> String {
        if !self.initialized {
            self.seed_last_id();
        }

        if self.pending.is_empty() {
            self.poll();
            if self.pending.is_empty() {
                rusty_tokio::time::sleep(self.poll_interval).await;
                return ": heartbeat\n\n".to_string();
            }
        }

        let chunk = self.pending.pop_front().expect("checked non-empty above");
        if self.pending.is_empty() {
            rusty_tokio::time::sleep(self.poll_interval).await;
        }
        chunk
    }
}

async fn stream_events(_req: Request) -> Response {
    // The `SseSource` closure is `FnMut`, called repeatedly -- an
    // owned Arc<Mutex<..>> clone per call (rather than a bare `&mut`
    // capture) is what lets each call's future be independently
    // `'static` + `Send` while still sharing one `last_id`/`pending`
    // across calls.
    let stream = Arc::new(rusty_tokio::sync::Mutex::new(
        TransformationEventStream::new(DEFAULT_DB_PATH.to_string()),
    ));
    Response::sse(Box::new(move || {
        let stream = stream.clone();
        Box::pin(async move {
            let mut guard = stream.lock_owned().await;
            guard.next_chunk().await
        })
    }))
}

/// Builds the `/transformation` router, bound to `state` for the
/// state/decisions/advance endpoints (the SSE endpoint takes no state
/// -- see the module doc).
pub fn router(state: Arc<AppState>) -> Router {
    let s = state.clone();
    let router = Router::new().get("/transformation/state", move |req| {
        let state = s.clone();
        async move { self::state(state, req).await }
    });

    let s = state.clone();
    let router = router.post("/transformation/decisions", move |req| {
        let state = s.clone();
        async move { create_decision(state, req).await }
    });

    let router = router.post("/transformation/advance", move |req| {
        let state = state.clone();
        async move { advance(state, req).await }
    });

    router.get("/transformation/events", |req| async move {
        stream_events(req).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::request::Request as HttpRequest;
    use rusty_http::{HeaderMap, Method};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempState {
        state: Arc<AppState>,
        path: PathBuf,
    }

    impl std::ops::Deref for TempState {
        type Target = Arc<AppState>;
        fn deref(&self) -> &Arc<AppState> {
            &self.state
        }
    }

    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn temp_state() -> TempState {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusty_meshed_transformation_test_{}_{n}.db",
            std::process::id()
        ));
        let conn = Connection::open(&path).unwrap();
        crate::models::ensure_schema(&conn).unwrap();
        crate::transformation::ensure_schema(&conn).unwrap();
        let mut state = AppState::new();
        state.set_engine(path.to_str().unwrap());
        TempState {
            state: Arc::new(state),
            path,
        }
    }

    fn req(method: Method, path: String, body: Json) -> HttpRequest {
        HttpRequest {
            method,
            path,
            query: Vec::new(),
            params: Vec::new(),
            headers: HeaderMap::new(),
            body: body.to_json_string().into_bytes(),
        }
    }

    #[rusty_tokio::test]
    async fn state_auto_seeds_and_returns_a_snapshot() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Get,
                "/transformation/state".to_string(),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(json.get("quarter").unwrap().as_f64(), Some(0.0));
        assert_eq!(
            json.get("legacy_systems")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[rusty_tokio::test]
    async fn create_decision_returns_201_for_a_valid_track_target() {
        let state = temp_state();
        let mut body = Json::object();
        body.insert("decision_type", "migrate_track");
        body.insert("target", "personnel-lifecycle");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/transformation/decisions".to_string(),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("decision_type").unwrap().as_str(),
            Some("migrate_track")
        );
        assert_eq!(
            json.get("target").unwrap().as_str(),
            Some("personnel-lifecycle")
        );
    }

    #[rusty_tokio::test]
    async fn create_decision_returns_404_for_an_unknown_track() {
        let state = temp_state();
        let mut body = Json::object();
        body.insert("decision_type", "migrate_track");
        body.insert("target", "no-such-track");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/transformation/decisions".to_string(),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some("Unknown track 'no-such-track'.")
        );
    }

    #[rusty_tokio::test]
    async fn create_decision_returns_422_for_an_invalid_global_target() {
        let state = temp_state();
        let mut body = Json::object();
        body.insert("decision_type", "invest_platform");
        body.insert("target", "not-a-real-target");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/transformation/decisions".to_string(),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[rusty_tokio::test]
    async fn create_decision_accepts_a_valid_global_target() {
        let state = temp_state();
        let mut body = Json::object();
        body.insert("decision_type", "invest_platform");
        body.insert("target", "platform");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/transformation/decisions".to_string(),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
    }

    #[rusty_tokio::test]
    async fn advance_returns_200_with_the_new_snapshot() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/transformation/advance".to_string(),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(json.get("quarter").unwrap().as_f64(), Some(1.0));
    }

    #[test]
    fn default_db_path_matches_the_source_constant() {
        assert_eq!(DEFAULT_DB_PATH, "meshed_registry.db");
    }

    fn temp_db_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rusty_meshed_transformation_events_test_{}_{n}.db",
            std::process::id()
        ))
    }

    #[rusty_tokio::test]
    async fn event_stream_emits_a_heartbeat_with_no_events() {
        let path = temp_db_path();
        let conn = Connection::open(&path).unwrap();
        crate::transformation::ensure_schema(&conn).unwrap();
        drop(conn);

        let mut stream = TransformationEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        let chunk = stream.next_chunk().await;
        assert_eq!(chunk, ": heartbeat\n\n");
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_yields_a_newly_recorded_row_as_a_data_event() {
        // XFM-031: only events *newer than connect-time* stream, so
        // the row is inserted after seed_last_id() runs against the
        // still-empty table -- see
        // event_stream_only_yields_events_newer_than_connect_time for
        // the same rule tested against a pre-existing "old" row.
        let path = temp_db_path();
        let conn = Connection::open(&path).unwrap();
        crate::transformation::ensure_schema(&conn).unwrap();

        let mut stream = TransformationEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        stream.seed_last_id();

        conn.execute(
            "INSERT INTO transformation_events (quarter, event_type, track, message, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                1,
                "decision_queued",
                "personnel-lifecycle",
                "queued migrate_track",
                "2026-01-01T00:00:00Z"
            ],
        )
        .unwrap();
        drop(conn);

        let chunk = stream.next_chunk().await;
        assert!(chunk.starts_with("data: "));
        assert!(chunk.contains("\"eventType\":\"decision_queued\""));
        assert!(chunk.contains("\"track\":\"personnel-lifecycle\""));
        assert!(chunk.ends_with("\n\n"));
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_only_yields_events_newer_than_connect_time() {
        let path = temp_db_path();
        let conn = Connection::open(&path).unwrap();
        crate::transformation::ensure_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO transformation_events (quarter, event_type, track, message, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                1,
                "old_event",
                "personnel-lifecycle",
                "before connect",
                "2026-01-01T00:00:00Z"
            ],
        )
        .unwrap();

        // Seed last_id at "connect time" (after the row above already
        // exists) before inserting a second, newer row.
        let mut stream = TransformationEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        stream.seed_last_id();

        conn.execute(
            "INSERT INTO transformation_events (quarter, event_type, track, message, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                1,
                "new_event",
                "personnel-lifecycle",
                "after connect",
                "2026-01-01T00:00:01Z"
            ],
        )
        .unwrap();
        drop(conn);

        let chunk = stream.next_chunk().await;
        assert!(chunk.contains("\"eventType\":\"new_event\""));
        assert!(!chunk.contains("old_event"));
        let _ = std::fs::remove_file(&path);
    }

    #[rusty_tokio::test]
    async fn event_stream_defaults_last_id_to_zero_when_db_missing() {
        let path = temp_db_path();
        // No file created at all -- Connection::open still succeeds
        // (SQLite creates it), but the table doesn't exist yet, so
        // MAX(id) errors and last_id must default to 0 (XFM-031/034).
        let mut stream = TransformationEventStream::with_poll_interval(
            path.to_str().unwrap().to_string(),
            Duration::from_millis(1),
        );
        stream.seed_last_id();
        assert_eq!(stream.last_id, 0);
        let _ = std::fs::remove_file(&path);
    }
}
