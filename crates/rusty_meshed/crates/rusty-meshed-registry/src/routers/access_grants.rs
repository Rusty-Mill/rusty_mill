//! Access-grant CRUD and RBAC-checked port resolution -- the Rust
//! port of `meshed.registry.routers.access_grants` (REG-087..103,
//! GOV-013..020). Two groups of endpoints, matching the source file's
//! own split:
//!
//! 1. `/access-grants` -- CRUD for [`PortAccessGrant`] records.
//! 2. `/data-products/{product_id}/output-ports/{port_id}/resolve` --
//!    returns an output port's topic name only if the requesting
//!    consumer group has an active grant (403 otherwise). GOV-01: a
//!    consumer without a grant is rejected before it can obtain the
//!    topic name to subscribe to.

use super::detail_error;
use crate::app::AppState;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use rusty_http::StatusCode;
use rusty_request::Json;
use rusty_sqlite::rusqlite::types::ToSql;
use rusty_sqlite::rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;

fn parse_id(req: &Request, name: &str) -> Option<i64> {
    req.param(name).and_then(|value| value.parse().ok())
}

fn bad_id(name: &str) -> Response {
    detail_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("{name} must be an integer"),
    )
}

fn internal_error() -> Response {
    detail_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

fn session_error() -> Response {
    detail_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Database engine is not initialized",
    )
}

fn product_not_found(product_id: i64) -> Response {
    detail_error(
        StatusCode::NOT_FOUND,
        format!("Data product {product_id} not found."),
    )
}

fn port_not_found(port_id: i64, product_id: i64) -> Response {
    detail_error(
        StatusCode::NOT_FOUND,
        format!("Output port {port_id} not found on data product {product_id}."),
    )
}

fn product_exists(conn: &Connection, product_id: i64) -> rusty_sqlite::rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM data_products WHERE id = ?1)",
        params![product_id],
        |row| row.get(0),
    )
}

fn output_port_exists(conn: &Connection, port_id: i64) -> rusty_sqlite::rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM output_ports WHERE id = ?1)",
        params![port_id],
        |row| row.get(0),
    )
}

fn grant_exists(
    conn: &Connection,
    output_port_id: i64,
    consumer_group_id: &str,
) -> rusty_sqlite::rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM port_access_grants WHERE output_port_id = ?1 AND consumer_group_id = ?2)",
        params![output_port_id, consumer_group_id],
        |row| row.get(0),
    )
}

// ---------------------------------------------------------------------
// Access grants CRUD
// ---------------------------------------------------------------------

async fn create(state: Arc<AppState>, req: Request) -> Response {
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    let Ok(body) = req.json() else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid JSON body");
    };
    let output_port_id = body.get("output_port_id").and_then(|v| v.as_f64());
    let consumer_group_id = body.get("consumer_group_id").and_then(|v| v.as_str());
    let granted_by = body.get("granted_by").and_then(|v| v.as_str());
    let (Some(output_port_id), Some(consumer_group_id), Some(granted_by)) =
        (output_port_id, consumer_group_id, granted_by)
    else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "output_port_id, consumer_group_id, and granted_by are all required",
        );
    };
    let output_port_id = output_port_id as i64;

    // GOV-014: port existence is checked before the duplicate check.
    match output_port_exists(&conn, output_port_id) {
        Ok(true) => {}
        Ok(false) => {
            return detail_error(
                StatusCode::NOT_FOUND,
                format!("Output port {output_port_id} not found."),
            )
        }
        Err(_) => return internal_error(),
    }
    // GOV-015: 409 on a duplicate (output_port_id, consumer_group_id) pair.
    match grant_exists(&conn, output_port_id, consumer_group_id) {
        Ok(true) => {
            return detail_error(
                StatusCode::CONFLICT,
                format!(
                    "Access grant for output_port_id={output_port_id} and \
                     consumer_group_id='{consumer_group_id}' already exists."
                ),
            )
        }
        Ok(false) => {}
        Err(_) => return internal_error(),
    }

    // REG-090: server-generated ISO-8601 UTC timestamp.
    let granted_at = now_iso();
    let inserted = conn.execute(
        "INSERT INTO port_access_grants (output_port_id, consumer_group_id, granted_by, granted_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![output_port_id, consumer_group_id, granted_by, granted_at],
    );
    if inserted.is_err() {
        return internal_error();
    }

    let mut json = Json::object();
    json.insert("id", conn.last_insert_rowid());
    json.insert("output_port_id", output_port_id);
    json.insert("consumer_group_id", consumer_group_id);
    json.insert("granted_by", granted_by);
    json.insert("granted_at", granted_at.as_str());
    Response::json(StatusCode::CREATED, &json)
}

async fn list(state: Arc<AppState>, req: Request) -> Response {
    let Ok(conn) = state.get_session() else {
        return session_error();
    };

    let output_port_id: Option<i64> = match req.query_param("output_port_id").map(str::parse) {
        Some(Ok(value)) => Some(value),
        Some(Err(_)) => {
            return detail_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "output_port_id must be an integer",
            )
        }
        None => None,
    };
    let consumer_group_id = req.query_param("consumer_group_id");

    // REG-092..096: every filter optional, AND-combined, no pagination.
    let mut sql =
        "SELECT id, output_port_id, consumer_group_id, granted_by, granted_at FROM port_access_grants"
            .to_string();
    let mut conditions: Vec<String> = Vec::new();
    let mut bindings: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(port_id) = output_port_id {
        conditions.push("output_port_id = ?".to_string());
        bindings.push(Box::new(port_id));
    }
    if let Some(consumer_group_id) = consumer_group_id {
        conditions.push("consumer_group_id = ?".to_string());
        bindings.push(Box::new(consumer_group_id.to_string()));
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    let Ok(mut stmt) = conn.prepare(&sql) else {
        return internal_error();
    };
    let param_refs: Vec<&dyn ToSql> = bindings.iter().map(AsRef::as_ref).collect();
    let Ok(rows) = stmt.query_map(param_refs.as_slice(), |row| {
        let id: i64 = row.get(0)?;
        let output_port_id: i64 = row.get(1)?;
        let consumer_group_id: String = row.get(2)?;
        let granted_by: String = row.get(3)?;
        let granted_at: String = row.get(4)?;
        Ok((
            id,
            output_port_id,
            consumer_group_id,
            granted_by,
            granted_at,
        ))
    }) else {
        return internal_error();
    };

    let mut grants = Json::array();
    for row in rows {
        let Ok((id, output_port_id, consumer_group_id, granted_by, granted_at)) = row else {
            return internal_error();
        };
        let mut json = Json::object();
        json.insert("id", id);
        json.insert("output_port_id", output_port_id);
        json.insert("consumer_group_id", consumer_group_id.as_str());
        json.insert("granted_by", granted_by.as_str());
        json.insert("granted_at", granted_at.as_str());
        grants.push(json);
    }
    Response::json(StatusCode::OK, &grants)
}

async fn revoke(state: Arc<AppState>, req: Request) -> Response {
    let Some(grant_id) = parse_id(&req, "id") else {
        return bad_id("id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match conn.execute(
        "DELETE FROM port_access_grants WHERE id = ?1",
        params![grant_id],
    ) {
        Ok(0) => detail_error(
            StatusCode::NOT_FOUND,
            format!("Access grant {grant_id} not found."),
        ),
        Ok(_) => Response::new(StatusCode::NO_CONTENT),
        Err(_) => internal_error(),
    }
}

// ---------------------------------------------------------------------
// RBAC-checked port resolution
// ---------------------------------------------------------------------

async fn resolve(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Some(port_id) = parse_id(&req, "port_id") else {
        return bad_id("port_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };

    // GOV-019: strict order -- product, then port ownership, then the
    // access-grant check runs last (only once both parents check out).
    match product_exists(&conn, product_id) {
        Ok(true) => {}
        Ok(false) => return product_not_found(product_id),
        Err(_) => return internal_error(),
    }

    let port = conn
        .query_row(
            "SELECT data_product_id, topic_name, schema_subject FROM output_ports WHERE id = ?1",
            params![port_id],
            |row| {
                let data_product_id: i64 = row.get(0)?;
                let topic_name: String = row.get(1)?;
                let schema_subject: String = row.get(2)?;
                Ok((data_product_id, topic_name, schema_subject))
            },
        )
        .optional();
    let (owning_product_id, topic_name, schema_subject) = match port {
        Ok(Some(port)) => port,
        Ok(None) => return port_not_found(port_id, product_id),
        Err(_) => return internal_error(),
    };
    if owning_product_id != product_id {
        return port_not_found(port_id, product_id);
    }

    // REG-103: required query param, 422 if omitted.
    let Some(consumer_group_id) = req.query_param("consumer_group_id") else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "consumer_group_id is required",
        );
    };

    match grant_exists(&conn, port_id, consumer_group_id) {
        Ok(true) => {}
        Ok(false) => {
            return detail_error(
                StatusCode::FORBIDDEN,
                format!(
                    "Consumer group '{consumer_group_id}' does not have access to output port {port_id}."
                ),
            )
        }
        Err(_) => return internal_error(),
    }

    let mut json = Json::object();
    json.insert("topic_name", topic_name.as_str());
    json.insert("schema_subject", schema_subject.as_str());
    Response::json(StatusCode::OK, &json)
}

/// A minimal RFC 3339 UTC "now" formatter -- same hand-rolled
/// civil-from-days algorithm (Howard Hinnant's) used elsewhere in this
/// crate family for a `created_at`/`granted_at` timestamp no test
/// asserts the exact value of; see `transformation::engine::now_iso`'s
/// doc for why there's no shared clock type to build on instead.
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

/// Builds the access-grant CRUD router plus the RBAC-checked resolve
/// endpoint, bound to `state` for DB access.
pub fn router(state: Arc<AppState>) -> Router {
    let s = state.clone();
    let router = Router::new().post("/access-grants", move |req| {
        let state = s.clone();
        async move { create(state, req).await }
    });

    let s = state.clone();
    let router = router.get("/access-grants", move |req| {
        let state = s.clone();
        async move { list(state, req).await }
    });

    let s = state.clone();
    let router = router.delete("/access-grants/{id}", move |req| {
        let state = s.clone();
        async move { revoke(state, req).await }
    });

    router.get(
        "/data-products/{product_id}/output-ports/{port_id}/resolve",
        move |req| {
            let state = state.clone();
            async move { resolve(state, req).await }
        },
    )
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
            "rusty_meshed_access_grants_test_{}_{n}.db",
            std::process::id()
        ));
        let conn = Connection::open(&path).unwrap();
        crate::models::ensure_schema(&conn).unwrap();
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

    fn create_product_and_port(state: &Arc<AppState>) -> (i64, i64) {
        let conn = state.get_session().unwrap();
        conn.execute(
            "INSERT INTO data_products (name, owner, version, domain, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["orders", "team-a", "1.0.0", "commerce", "Order events"],
        )
        .unwrap();
        let product_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type) VALUES (?1, ?2, ?3, ?4)",
            params![product_id, "orders.created", "orders.created-value", "delta"],
        )
        .unwrap();
        (product_id, conn.last_insert_rowid())
    }

    fn grant_body(output_port_id: i64) -> Json {
        let mut body = Json::object();
        body.insert("output_port_id", output_port_id);
        body.insert("consumer_group_id", "billing-service");
        body.insert("granted_by", "admin@example.com");
        body
    }

    #[rusty_tokio::test]
    async fn create_returns_201_with_assigned_id_and_granted_at() {
        let state = temp_state();
        let (_product_id, port_id) = create_product_and_port(&state);
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/access-grants".to_string(),
                grant_body(port_id),
            ))
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(json.get("id").unwrap().as_f64().unwrap() > 0.0);
        assert!(!json.get("granted_at").unwrap().as_str().unwrap().is_empty());
    }

    #[rusty_tokio::test]
    async fn create_returns_404_when_port_missing() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/access-grants".to_string(),
                grant_body(999),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some("Output port 999 not found.")
        );
    }

    #[rusty_tokio::test]
    async fn create_returns_409_on_duplicate_port_and_consumer_group() {
        let state = temp_state();
        let (_product_id, port_id) = create_product_and_port(&state);
        let r = router((*state).clone());
        r.dispatch(req(
            Method::Post,
            "/access-grants".to_string(),
            grant_body(port_id),
        ))
        .await;
        let response = r
            .dispatch(req(
                Method::Post,
                "/access-grants".to_string(),
                grant_body(port_id),
            ))
            .await;
        assert_eq!(response.status, StatusCode::CONFLICT);
    }

    #[rusty_tokio::test]
    async fn list_returns_all_grants_with_no_filters() {
        let state = temp_state();
        let (_product_id, port_id) = create_product_and_port(&state);
        let r = router((*state).clone());
        r.dispatch(req(
            Method::Post,
            "/access-grants".to_string(),
            grant_body(port_id),
        ))
        .await;

        let response = r
            .dispatch(req(
                Method::Get,
                "/access-grants".to_string(),
                Json::object(),
            ))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);
    }

    #[rusty_tokio::test]
    async fn list_filters_by_output_port_id_and_consumer_group_id_additively() {
        let state = temp_state();
        let (_product_id, port_a) = create_product_and_port(&state);
        let (_product_id_2, port_b) = create_product_and_port(&state);
        let r = router((*state).clone());
        r.dispatch(req(
            Method::Post,
            "/access-grants".to_string(),
            grant_body(port_a),
        ))
        .await;
        let mut other = grant_body(port_b);
        other.insert("consumer_group_id", "analytics");
        r.dispatch(req(Method::Post, "/access-grants".to_string(), other))
            .await;

        let mut request = req(Method::Get, "/access-grants".to_string(), Json::object());
        request
            .query
            .push(("output_port_id".to_string(), port_a.to_string()));
        request.query.push((
            "consumer_group_id".to_string(),
            "billing-service".to_string(),
        ));
        let response = r.dispatch(request).await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let grants = json.as_array().unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(
            grants[0].get("output_port_id").unwrap().as_f64(),
            Some(port_a as f64)
        );
    }

    #[rusty_tokio::test]
    async fn revoke_returns_204() {
        let state = temp_state();
        let (_product_id, port_id) = create_product_and_port(&state);
        let r = router((*state).clone());
        let create_response = r
            .dispatch(req(
                Method::Post,
                "/access-grants".to_string(),
                grant_body(port_id),
            ))
            .await;
        let created = Json::parse(std::str::from_utf8(&create_response.body).unwrap()).unwrap();
        let grant_id = created.get("id").unwrap().as_f64().unwrap() as i64;

        let response = r
            .dispatch(req(
                Method::Delete,
                format!("/access-grants/{grant_id}"),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
    }

    #[rusty_tokio::test]
    async fn revoke_returns_404_when_missing() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Delete,
                "/access-grants/999".to_string(),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some("Access grant 999 not found.")
        );
    }

    #[rusty_tokio::test]
    async fn resolve_returns_topic_and_schema_subject_with_an_active_grant() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let r = router((*state).clone());
        r.dispatch(req(
            Method::Post,
            "/access-grants".to_string(),
            grant_body(port_id),
        ))
        .await;

        let mut request = req(
            Method::Get,
            format!("/data-products/{product_id}/output-ports/{port_id}/resolve"),
            Json::object(),
        );
        request.query.push((
            "consumer_group_id".to_string(),
            "billing-service".to_string(),
        ));
        let response = r.dispatch(request).await;
        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("topic_name").unwrap().as_str(),
            Some("orders.created")
        );
        assert_eq!(
            json.get("schema_subject").unwrap().as_str(),
            Some("orders.created-value")
        );
    }

    #[rusty_tokio::test]
    async fn resolve_returns_403_with_no_matching_grant() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let mut request = req(
            Method::Get,
            format!("/data-products/{product_id}/output-ports/{port_id}/resolve"),
            Json::object(),
        );
        request.query.push((
            "consumer_group_id".to_string(),
            "billing-service".to_string(),
        ));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some(format!(
                "Consumer group 'billing-service' does not have access to output port {port_id}."
            ))
            .as_deref()
        );
    }

    #[rusty_tokio::test]
    async fn resolve_returns_404_if_product_missing_checked_before_port() {
        let state = temp_state();
        let mut request = req(
            Method::Get,
            "/data-products/999/output-ports/1/resolve".to_string(),
            Json::object(),
        );
        request.query.push((
            "consumer_group_id".to_string(),
            "billing-service".to_string(),
        ));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some("Data product 999 not found.")
        );
    }

    #[rusty_tokio::test]
    async fn resolve_returns_404_if_port_missing_or_mismatched() {
        let state = temp_state();
        let (product_id, _port_id) = create_product_and_port(&state);
        let mut request = req(
            Method::Get,
            format!("/data-products/{product_id}/output-ports/999/resolve"),
            Json::object(),
        );
        request.query.push((
            "consumer_group_id".to_string(),
            "billing-service".to_string(),
        ));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn resolve_returns_422_when_consumer_group_id_omitted() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let request = req(
            Method::Get,
            format!("/data-products/{product_id}/output-ports/{port_id}/resolve"),
            Json::object(),
        );
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
