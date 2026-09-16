//! Lineage query endpoints -- the Rust port of
//! `meshed.registry.routers.lineage` (REG-107..109). Two read-only
//! endpoints backed by [`LineageTracker`] (already built in
//! `rusty-meshed-observability`): this router is pure wiring, no new
//! lineage logic.
//!
//! Unlike every other router in this crate, this one takes no
//! [`crate::app::AppState`] at all: it doesn't read the registry's own
//! configured DB path, it opens [`DEFAULT_DB_PATH`] directly -- a
//! hardcoded `"meshed_registry.db"`, not sourced from `PlatformConfig`
//! or the environment (REG-109). That's a real quirk of the source,
//! preserved deliberately rather than "fixed": the Python router
//! constructs its own `LineageTracker(db_path=_DEFAULT_DB_PATH)`
//! rather than taking the app's injected session, and the manifest
//! row exists specifically to capture that as tested, intentional
//! behavior.

use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use rusty_meshed_observability::LineageTracker;
use rusty_request::Json;

/// The source's `_DEFAULT_DB_PATH` module constant (REG-109) --
/// hardcoded, not read from [`crate::app::AppState`] or
/// [`rusty_meshed_core::PlatformConfig`].
pub const DEFAULT_DB_PATH: &str = "meshed_registry.db";

fn internal_error() -> Response {
    Response::text(
        rusty_http::StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error",
    )
}

async fn topology_response(db_path: &str) -> Response {
    let Ok(tracker) = LineageTracker::new(db_path) else {
        return internal_error();
    };
    let Ok(dependencies) = tracker.get_topology_dependencies() else {
        return internal_error();
    };

    let mut dependencies_array = Json::array();
    for dependency in dependencies {
        let mut entry = Json::object();
        entry.insert("consumer", dependency.consumer.as_str());
        entry.insert("input_topic", dependency.input_topic.as_str());
        dependencies_array.push(entry);
    }
    let mut body = Json::object();
    body.insert("dependencies", dependencies_array);
    Response::json(rusty_http::StatusCode::OK, &body)
}

async fn record_lineage_response(db_path: &str, correlation_id: &str) -> Response {
    let Ok(tracker) = LineageTracker::new(db_path) else {
        return internal_error();
    };
    // Empty events, not 404, for an unknown correlation_id -- absence
    // of lineage is not an error (matches LineageTracker's own
    // behavior one layer down).
    let Ok(records) = tracker.get_record_lineage(correlation_id) else {
        return internal_error();
    };

    let mut events = Json::array();
    for record in records {
        let mut source_event_ids = Json::array();
        for id in &record.source_event_ids {
            source_event_ids.push(id.as_str());
        }
        let mut entry = Json::object();
        entry.insert("event_id", record.event_id.as_str());
        entry.insert("correlation_id", record.correlation_id.as_str());
        entry.insert("source_event_ids", source_event_ids);
        entry.insert("product_name", record.product_name.as_str());
        entry.insert("topic_name", record.topic_name.as_str());
        entry.insert("event_timestamp", record.event_timestamp.as_str());
        events.push(entry);
    }
    let mut body = Json::object();
    body.insert("correlation_id", correlation_id);
    body.insert("events", events);
    Response::json(rusty_http::StatusCode::OK, &body)
}

async fn get_topology(_req: Request) -> Response {
    topology_response(DEFAULT_DB_PATH).await
}

async fn get_record_lineage(req: Request) -> Response {
    let correlation_id = req.param("correlation_id").unwrap_or("").to_string();
    record_lineage_response(DEFAULT_DB_PATH, &correlation_id).await
}

/// Builds the `/lineage` router. Takes no `AppState` -- see the
/// module doc for why.
pub fn router() -> Router {
    Router::new()
        .get(
            "/lineage/topology",
            |req| async move { get_topology(req).await },
        )
        .get("/lineage/record/{correlation_id}", |req| async move {
            get_record_lineage(req).await
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Tests exercise [`topology_response`]/[`record_lineage_response`]
    /// directly against a real temporary file rather than dispatching
    /// through [`router`] (which always opens the literal
    /// `DEFAULT_DB_PATH`, a relative path in the process's current
    /// working directory) -- going through the router in a test would
    /// either collide with other tests running in parallel against the
    /// same relative path, or leave a stray `meshed_registry.db` file
    /// behind in the repo. [`default_db_path_matches_the_source_constant`]
    /// below locks in REG-109's literal value separately.
    struct TempPath(PathBuf);

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn temp_db_path() -> TempPath {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        TempPath(std::env::temp_dir().join(format!(
            "rusty_meshed_lineage_router_test_{}_{n}.db",
            std::process::id()
        )))
    }

    #[test]
    fn default_db_path_matches_the_source_constant() {
        assert_eq!(DEFAULT_DB_PATH, "meshed_registry.db");
    }

    #[rusty_tokio::test]
    async fn topology_returns_dependencies_recorded_by_the_tracker() {
        let path = temp_db_path();
        let db_path = path.0.to_str().unwrap();
        let tracker = LineageTracker::new(db_path).unwrap();
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

        let response = topology_response(db_path).await;
        assert_eq!(response.status, rusty_http::StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let dependencies = body.get("dependencies").unwrap().as_array().unwrap();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(
            dependencies[0].get("consumer").unwrap().as_str(),
            Some("readiness-reporting")
        );
    }

    #[rusty_tokio::test]
    async fn topology_is_an_empty_list_with_no_recorded_events() {
        let path = temp_db_path();
        let response = topology_response(path.0.to_str().unwrap()).await;
        assert_eq!(response.status, rusty_http::StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(body
            .get("dependencies")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[rusty_tokio::test]
    async fn record_lineage_returns_events_for_a_known_correlation_id() {
        let path = temp_db_path();
        let db_path = path.0.to_str().unwrap();
        let tracker = LineageTracker::new(db_path).unwrap();
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

        let response = record_lineage_response(db_path, "corr-1").await;
        assert_eq!(response.status, rusty_http::StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(body.get("correlation_id").unwrap().as_str(), Some("corr-1"));
        let events = body.get("events").unwrap().as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("event_id").unwrap().as_str(), Some("event-1"));
    }

    #[rusty_tokio::test]
    async fn record_lineage_returns_200_with_empty_events_for_an_unknown_correlation_id() {
        let path = temp_db_path();
        let response = record_lineage_response(path.0.to_str().unwrap(), "no-such-id").await;
        assert_eq!(response.status, rusty_http::StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            body.get("correlation_id").unwrap().as_str(),
            Some("no-such-id")
        );
        assert!(body.get("events").unwrap().as_array().unwrap().is_empty());
    }

    #[rusty_tokio::test]
    async fn router_dispatches_both_routes_by_pattern() {
        // Confirms the route table shape (method + pattern) without
        // touching the real DEFAULT_DB_PATH file.
        let routes = router().routes();
        assert_eq!(
            routes,
            vec![
                (rusty_http::Method::Get, "/lineage/topology".to_string()),
                (
                    rusty_http::Method::Get,
                    "/lineage/record/{correlation_id}".to_string()
                ),
            ]
        );
    }
}
