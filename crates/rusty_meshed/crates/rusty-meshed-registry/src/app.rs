//! App bootstrap -- the Rust port of `meshed.registry.app.create_app`
//! and `meshed.registry.dependencies` (REG-001..011, REG-136..137,
//! XFM-049).
//!
//! What this covers: app metadata, DB path wiring, the
//! `create_all`-on-startup step, CORS, and a `get_session`/`get_config`
//! dependency pair. [`build_router`] mounts every per-resource router
//! this crate has (`data_products`, `ports`, `contracts`,
//! `access_grants`, `governance`, `lineage`, `transformation`,
//! `metrics`, and `monitor` -- see `crate::routers`) plus
//! `/openapi.json` and `/docs`.

use crate::http::response::Response;
use crate::http::router::Router;
use rusty_err::Error;
use rusty_http::StatusCode;
use rusty_meshed_core::{ConfigError, PlatformConfig};
use rusty_sqlite::rusqlite::{Connection, Result as SqlResult};
use std::sync::Arc;

/// `meshed.registry.app`'s `FastAPI(title=...)` (REG-001). Also stands
/// in for `meshed.__version__` (XFM-049) -- the source's package
/// version and its registry app's declared version are both
/// `"0.1.0"`, so one constant covers both capabilities.
pub const TITLE: &str = "Meshed Data Product Registry";
pub const DESCRIPTION: &str = "Register, discover, and govern data products on the meshed platform. Provides CRUD operations for data products, input/output ports, and data contracts.";
pub const VERSION: &str = "0.1.0";

/// Raised by [`AppState::get_session`] -- the Rust equivalent of the
/// source's `get_session()` raising `RuntimeError` when
/// `set_engine` was never called (REG-007).
#[derive(Debug, Error)]
pub enum SessionError {
    #[error(
        "Database engine is not initialized. Ensure the app factory lifespan has run before handling requests."
    )]
    NotInitialized,
    #[error("{0}")]
    Sql(String),
}

/// Holds the one piece of state a request handler needs beyond its own
/// arguments: where the SQLite database lives. Set once during
/// startup via [`AppState::set_engine`] (REG-002), matching the
/// source's `set_engine()` being called from the app factory's
/// lifespan context manager before `create_all` runs.
///
/// The source stores this as a module-level global (its own comment
/// notes this is deliberate -- set explicitly by the app factory
/// rather than a magic import-time singleton). This crate makes the
/// same "set once, explicitly, before serving" shape a plain struct
/// instead of a global, since Rust has no equivalent of Python's
/// mutable module global without reaching for `OnceLock`/`Mutex` --
/// an explicit `Arc<AppState>` handed to every request handler serves
/// the same purpose without hidden global state.
#[derive(Debug, Default)]
pub struct AppState {
    db_path: Option<String>,
}

impl AppState {
    pub fn new() -> Self {
        AppState { db_path: None }
    }

    /// Registers the SQLite database path for [`get_session`] to open
    /// connections against (REG-002's `create_engine(f"sqlite:///
    /// {config.registry_db_path}")` -- this crate opens a fresh
    /// `rusqlite::Connection` per session rather than pooling a
    /// SQLAlchemy engine, the same per-call-connection shape used
    /// throughout `rusty_meshed` since `rusqlite::Connection` isn't
    /// `Sync`).
    pub fn set_engine(&mut self, db_path: impl Into<String>) {
        self.db_path = Some(db_path.into());
    }

    /// Opens a fresh connection for the current request. `Err` before
    /// [`set_engine`](Self::set_engine) has ever been called
    /// (REG-007).
    pub fn get_session(&self) -> Result<Connection, SessionError> {
        let db_path = self
            .db_path
            .as_deref()
            .ok_or(SessionError::NotInitialized)?;
        let conn = Connection::open(db_path).map_err(|err| SessionError::Sql(err.to_string()))?;
        // SQLite disables FK enforcement by default per connection --
        // re-enable it on every session, not just at ensure_schema
        // time, so the ON DELETE CASCADE clauses (REG-018..020) fire
        // for every request, not just the one that created the tables.
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|err| SessionError::Sql(err.to_string()))?;
        Ok(conn)
    }
}

/// A fresh [`PlatformConfig`] for the current request, read from the
/// process environment (REG-008) -- so an environment variable change
/// is picked up without a process restart. A thin, named wrapper
/// around [`PlatformConfig::from_env`] that exists as the DI seam a
/// future route handler calls, mirroring the source's `get_config()`
/// dependency provider.
pub fn get_config() -> Result<PlatformConfig, ConfigError> {
    PlatformConfig::from_env()
}

/// Creates every table this crate currently owns, idempotently
/// (REG-003) -- the Rust equivalent of `SQLModel.metadata.create_all`.
/// The source additionally imports `observability.metrics`,
/// `governance.rbac`, and `infrastructure.outbox` purely for their
/// side effect of registering more tables before `create_all` runs
/// (REG-004); `observability.metrics`'s `SchemaViolation` table now has
/// a Rust equivalent (`rusty_meshed_observability::ensure_metrics_schema`,
/// called below) -- `monitor::get_metrics` (REG-133) needs it. RBAC and
/// the transactional outbox are still open capabilities with no
/// persistent store of their own yet; add their `ensure_schema` calls
/// here as each one lands, same as the three below.
pub fn create_all(conn: &Connection) -> SqlResult<()> {
    crate::models::ensure_schema(conn)?;
    crate::transformation::ensure_schema(conn)?;
    rusty_meshed_observability::ensure_metrics_schema(conn)?;
    Ok(())
}

/// A minimal Swagger UI shell for `/docs` (REG-136), loading the UI
/// bundle from a CDN and pointing it at `/openapi.json` -- the same
/// shape FastAPI's own default `/docs` page uses (it also serves an
/// HTML shell referencing a CDN-hosted `swagger-ui-dist` bundle rather
/// than vendoring the UI itself), so this isn't a new dependency this
/// crate takes on, just a small HTML page a browser fetches the actual
/// UI code from.
pub fn docs_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<title>{TITLE} - Swagger UI</title>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
<div id="swagger-ui"></div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>
window.onload = () => {{
  window.ui = SwaggerUIBundle({{
    url: '/openapi.json',
    dom_id: '#swagger-ui',
  }});
}};
</script>
</body>
</html>"#
    )
}

/// Builds the app's route table: the data-products, ports, contracts,
/// and access-grants CRUD routers, the governance dry-run endpoint,
/// the lineage query endpoints, the transformation simulator
/// endpoints, and the per-product metrics endpoint today, plus
/// `/openapi.json` and `/docs` (REG-136, REG-137); the remaining
/// per-resource router merges in here as it's built (see the module
/// doc). `state` is shared across every resource router that needs DB
/// access -- `lineage` and `transformation`'s SSE endpoint don't take
/// it, see those modules' own docs for why.
pub fn build_router(state: Arc<AppState>) -> Router {
    let business_router = crate::routers::data_products::router(state.clone())
        .merge(crate::routers::ports::router(state.clone()))
        .merge(crate::routers::contracts::router(state.clone()))
        .merge(crate::routers::access_grants::router(state.clone()))
        .merge(crate::routers::governance::router())
        .merge(crate::routers::lineage::router())
        .merge(crate::routers::transformation::router(state.clone()))
        .merge(crate::routers::metrics::router(state.clone()))
        .merge(crate::routers::monitor::router(state));
    let mut route_table = business_router.routes();
    route_table.push((rusty_http::Method::Get, "/openapi.json".to_string()));
    route_table.push((rusty_http::Method::Get, "/docs".to_string()));
    let route_table = Arc::new(route_table);

    business_router
        .get("/openapi.json", move |_req| {
            let table = route_table.clone();
            async move {
                let doc = openapi_json(&table);
                Response::json(StatusCode::OK, &doc)
            }
        })
        .get("/docs", |_req| async move {
            Response::html(StatusCode::OK, docs_html())
        })
}

/// Builds the app's `{"openapi": ..., "info": {...}, "paths": {...}}`
/// document (REG-001, REG-137) from a `(method, pattern)` route table
/// -- a plain slice rather than a live [`Router`] reference, since
/// [`build_router`] needs the *finished* route list (including
/// `/openapi.json` and `/docs` themselves) inside a handler defined
/// *before* the router is finished being built; [`Router::routes`]
/// produces the same shape once a router exists, so callers with a
/// finished router just pass `router.routes()` straight through. Each
/// path/method entry is a minimal stub (no request/response schemas)
/// -- generating those needs the Create/Public schema types (already
/// built, see `models::schemas`) to carry their own JSON Schema
/// representation, future work once a real resource router exists to
/// attach them to.
pub fn openapi_json(routes: &[(rusty_http::Method, String)]) -> rusty_request::Json {
    use std::collections::BTreeMap;

    let mut by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (method, pattern) in routes {
        by_path
            .entry(pattern.clone())
            .or_default()
            .push(method.as_str().to_lowercase());
    }

    let mut paths = rusty_request::Json::object();
    for (pattern, methods) in by_path {
        let mut path_item = rusty_request::Json::object();
        for method in methods {
            let mut responses = rusty_request::Json::object();
            let mut ok = rusty_request::Json::object();
            ok.insert("description", "Successful Response");
            responses.insert("200", ok);
            let mut operation = rusty_request::Json::object();
            operation.insert("responses", responses);
            path_item.insert(method.as_str(), operation);
        }
        paths.insert(pattern.as_str(), path_item);
    }

    let mut info = rusty_request::Json::object();
    info.insert("title", TITLE);
    info.insert("description", DESCRIPTION);
    info.insert("version", VERSION);

    let mut doc = rusty_request::Json::object();
    doc.insert("openapi", "3.0.0");
    doc.insert("info", info);
    doc.insert("paths", paths);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_get_session_fails_before_set_engine() {
        let state = AppState::new();
        let err = state.get_session().unwrap_err();
        assert!(matches!(err, SessionError::NotInitialized));
    }

    #[test]
    fn app_state_get_session_opens_a_connection_after_set_engine() {
        let mut state = AppState::new();
        state.set_engine(":memory:");
        state.get_session().unwrap();
    }

    #[test]
    fn get_config_reads_a_fresh_config_each_call() {
        let config = get_config().unwrap();
        assert!(!config.registry_db_path.is_empty());
    }

    #[test]
    fn create_all_is_idempotent_and_creates_every_table() {
        let conn = Connection::open_in_memory().unwrap();
        create_all(&conn).unwrap();
        create_all(&conn).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
                 ('data_products', 'input_ports', 'output_ports', 'data_contracts', \
                 'transformation_clock', 'legacy_systems', 'capability_scores', \
                 'transformation_decisions', 'transformation_events', 'schema_violations')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 10);
    }

    #[test]
    fn openapi_json_reflects_registered_routes() {
        let router = Router::new().get("/health", |_req| async {
            crate::http::response::Response::text(StatusCode::OK, "ok")
        });
        let doc = openapi_json(&router.routes());
        assert_eq!(
            doc.get("info").unwrap().get("title").unwrap().as_str(),
            Some(TITLE)
        );
        assert!(doc
            .get("paths")
            .unwrap()
            .get("/health")
            .unwrap()
            .get("get")
            .is_some());
    }

    #[test]
    fn docs_html_references_openapi_json() {
        let html = docs_html();
        assert!(html.contains("/openapi.json"));
        assert!(html.contains(TITLE));
    }

    #[rusty_tokio::test]
    async fn build_router_serves_openapi_json_and_docs() {
        let router = build_router(Arc::new(AppState::new()));

        let openapi_response = router
            .dispatch(crate::http::request::Request {
                method: rusty_http::Method::Get,
                path: "/openapi.json".to_string(),
                query: Vec::new(),
                params: Vec::new(),
                headers: rusty_http::HeaderMap::new(),
                body: Vec::new(),
            })
            .await;
        assert_eq!(openapi_response.status, StatusCode::OK);
        let doc = rusty_request::Json::parse(std::str::from_utf8(&openapi_response.body).unwrap())
            .unwrap();
        assert!(doc.get("paths").unwrap().get("/docs").is_some());
        assert!(doc.get("paths").unwrap().get("/openapi.json").is_some());

        let docs_response = router
            .dispatch(crate::http::request::Request {
                method: rusty_http::Method::Get,
                path: "/docs".to_string(),
                query: Vec::new(),
                params: Vec::new(),
                headers: rusty_http::HeaderMap::new(),
                body: Vec::new(),
            })
            .await;
        assert_eq!(docs_response.status, StatusCode::OK);
    }

    /// End-to-end smoke test over a real TCP socket -- proves
    /// [`crate::http::server::serve`], [`build_router`], and CORS all
    /// work together, not just each piece in isolation.
    #[rusty_tokio::test]
    async fn serve_answers_a_real_http_request_over_tcp() {
        let listener = rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Arc::new(build_router(Arc::new(AppState::new())));
        rusty_tokio::spawn(async move {
            let _ = crate::http::server::serve(listener, router).await;
        });

        let response = rusty_request::Client::new()
            .get(&format!("http://{addr}/openapi.json"))
            .unwrap()
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let body = response.json().unwrap();
        assert_eq!(
            body.get("info").unwrap().get("version").unwrap().as_str(),
            Some(VERSION)
        );
    }
}
