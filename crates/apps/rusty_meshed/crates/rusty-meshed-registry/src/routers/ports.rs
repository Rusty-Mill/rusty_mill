//! Input and output port CRUD endpoints -- the Rust port of
//! `meshed.registry.routers.ports` (REG-059..075). Both port kinds
//! are handled in one module because the source does too: they're
//! structurally identical (create/list/delete scoped to a parent data
//! product, with the same 404-on-missing-parent guard) and differ
//! only in which fields a port carries.

use super::detail_error;
use crate::app::AppState;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use rusty_http::StatusCode;
use rusty_meshed_core::EventType;
use rusty_request::Json;
use rusty_sqlite::rusqlite::{params, Connection};
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

/// The exact wording (including trailing period) the source's
/// `_get_product_or_404` uses -- distinct from `data_products`
/// router's own "Data product not found" (no id, no period).
fn product_not_found(product_id: i64) -> Response {
    detail_error(
        StatusCode::NOT_FOUND,
        format!("Data product {product_id} not found."),
    )
}

fn product_exists(conn: &Connection, product_id: i64) -> rusty_sqlite::rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM data_products WHERE id = ?1)",
        params![product_id],
        |row| row.get(0),
    )
}

// ---------------------------------------------------------------------
// Input port endpoints
// ---------------------------------------------------------------------

async fn create_input_port(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match product_exists(&conn, product_id) {
        Ok(true) => {}
        Ok(false) => return product_not_found(product_id),
        Err(_) => return internal_error(),
    }

    let Ok(body) = req.json() else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid JSON body");
    };
    let Some(topic_name) = body.get("topic_name").and_then(|v| v.as_str()) else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "topic_name is required");
    };
    let description = body.get("description").and_then(|v| v.as_str());

    let inserted = conn.execute(
        "INSERT INTO input_ports (data_product_id, topic_name, description) VALUES (?1, ?2, ?3)",
        params![product_id, topic_name, description],
    );
    if inserted.is_err() {
        return internal_error();
    }

    let mut json = Json::object();
    json.insert("id", conn.last_insert_rowid());
    json.insert("data_product_id", product_id);
    json.insert("topic_name", topic_name);
    json.insert("description", description);
    Response::json(StatusCode::CREATED, &json)
}

async fn list_input_ports(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match product_exists(&conn, product_id) {
        Ok(true) => {}
        Ok(false) => return product_not_found(product_id),
        Err(_) => return internal_error(),
    }

    let Ok(mut stmt) = conn.prepare(
        "SELECT id, data_product_id, topic_name, description FROM input_ports WHERE data_product_id = ?1",
    ) else {
        return internal_error();
    };
    let Ok(rows) = stmt.query_map(params![product_id], |row| {
        let id: i64 = row.get(0)?;
        let data_product_id: i64 = row.get(1)?;
        let topic_name: String = row.get(2)?;
        let description: Option<String> = row.get(3)?;
        Ok((id, data_product_id, topic_name, description))
    }) else {
        return internal_error();
    };

    let mut ports = Json::array();
    for row in rows {
        let Ok((id, data_product_id, topic_name, description)) = row else {
            return internal_error();
        };
        let mut json = Json::object();
        json.insert("id", id);
        json.insert("data_product_id", data_product_id);
        json.insert("topic_name", topic_name.as_str());
        json.insert("description", description);
        ports.push(json);
    }
    Response::json(StatusCode::OK, &ports)
}

async fn delete_input_port(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Some(port_id) = parse_id(&req, "port_id") else {
        return bad_id("port_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match product_exists(&conn, product_id) {
        Ok(true) => {}
        Ok(false) => return product_not_found(product_id),
        Err(_) => return internal_error(),
    }

    match conn.execute(
        "DELETE FROM input_ports WHERE id = ?1 AND data_product_id = ?2",
        params![port_id, product_id],
    ) {
        Ok(0) => detail_error(
            StatusCode::NOT_FOUND,
            format!("Input port {port_id} not found on data product {product_id}."),
        ),
        Ok(_) => Response::new(StatusCode::NO_CONTENT),
        Err(_) => internal_error(),
    }
}

// ---------------------------------------------------------------------
// Output port endpoints
// ---------------------------------------------------------------------

async fn create_output_port(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match product_exists(&conn, product_id) {
        Ok(true) => {}
        Ok(false) => return product_not_found(product_id),
        Err(_) => return internal_error(),
    }

    let Ok(body) = req.json() else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid JSON body");
    };
    let Some(topic_name) = body.get("topic_name").and_then(|v| v.as_str()) else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "topic_name is required");
    };
    let Some(schema_subject) = body.get("schema_subject").and_then(|v| v.as_str()) else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "schema_subject is required",
        );
    };
    let Some(event_type_raw) = body.get("event_type").and_then(|v| v.as_str()) else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "event_type is required");
    };
    // REG-070: an invalid event_type is a 422, not a 500 or a silent fallback.
    let Some(event_type) = EventType::parse(event_type_raw) else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("'{event_type_raw}' is not a valid event_type"),
        );
    };
    let description = body.get("description").and_then(|v| v.as_str());

    let inserted = conn.execute(
        "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type, description) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![product_id, topic_name, schema_subject, event_type.as_str(), description],
    );
    if inserted.is_err() {
        return internal_error();
    }

    let mut json = Json::object();
    json.insert("id", conn.last_insert_rowid());
    json.insert("data_product_id", product_id);
    json.insert("topic_name", topic_name);
    json.insert("schema_subject", schema_subject);
    json.insert("event_type", event_type.as_str());
    json.insert("description", description);
    Response::json(StatusCode::CREATED, &json)
}

async fn list_output_ports(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match product_exists(&conn, product_id) {
        Ok(true) => {}
        Ok(false) => return product_not_found(product_id),
        Err(_) => return internal_error(),
    }

    let Ok(mut stmt) = conn.prepare(
        "SELECT id, data_product_id, topic_name, schema_subject, event_type, description \
         FROM output_ports WHERE data_product_id = ?1",
    ) else {
        return internal_error();
    };
    let Ok(rows) = stmt.query_map(params![product_id], |row| {
        let id: i64 = row.get(0)?;
        let data_product_id: i64 = row.get(1)?;
        let topic_name: String = row.get(2)?;
        let schema_subject: String = row.get(3)?;
        let event_type: String = row.get(4)?;
        let description: Option<String> = row.get(5)?;
        Ok((
            id,
            data_product_id,
            topic_name,
            schema_subject,
            event_type,
            description,
        ))
    }) else {
        return internal_error();
    };

    let mut ports = Json::array();
    for row in rows {
        let Ok((id, data_product_id, topic_name, schema_subject, event_type, description)) = row
        else {
            return internal_error();
        };
        let mut json = Json::object();
        json.insert("id", id);
        json.insert("data_product_id", data_product_id);
        json.insert("topic_name", topic_name.as_str());
        json.insert("schema_subject", schema_subject.as_str());
        json.insert("event_type", event_type.as_str());
        json.insert("description", description);
        ports.push(json);
    }
    Response::json(StatusCode::OK, &ports)
}

async fn delete_output_port(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Some(port_id) = parse_id(&req, "port_id") else {
        return bad_id("port_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    match product_exists(&conn, product_id) {
        Ok(true) => {}
        Ok(false) => return product_not_found(product_id),
        Err(_) => return internal_error(),
    }

    match conn.execute(
        "DELETE FROM output_ports WHERE id = ?1 AND data_product_id = ?2",
        params![port_id, product_id],
    ) {
        Ok(0) => detail_error(
            StatusCode::NOT_FOUND,
            format!("Output port {port_id} not found on data product {product_id}."),
        ),
        Ok(_) => Response::new(StatusCode::NO_CONTENT),
        Err(_) => internal_error(),
    }
}

/// Builds the input/output port router, bound to `state` for DB
/// access.
pub fn router(state: Arc<AppState>) -> Router {
    let s = state.clone();
    let router = Router::new().post("/data-products/{product_id}/input-ports", move |req| {
        let state = s.clone();
        async move { create_input_port(state, req).await }
    });

    let s = state.clone();
    let router = router.get("/data-products/{product_id}/input-ports", move |req| {
        let state = s.clone();
        async move { list_input_ports(state, req).await }
    });

    let s = state.clone();
    let router = router.delete(
        "/data-products/{product_id}/input-ports/{port_id}",
        move |req| {
            let state = s.clone();
            async move { delete_input_port(state, req).await }
        },
    );

    let s = state.clone();
    let router = router.post("/data-products/{product_id}/output-ports", move |req| {
        let state = s.clone();
        async move { create_output_port(state, req).await }
    });

    let s = state.clone();
    let router = router.get("/data-products/{product_id}/output-ports", move |req| {
        let state = s.clone();
        async move { list_output_ports(state, req).await }
    });

    router.delete(
        "/data-products/{product_id}/output-ports/{port_id}",
        move |req| {
            let state = state.clone();
            async move { delete_output_port(state, req).await }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::request::Request as HttpRequest;
    use rusty_http::{HeaderMap, Method};
    use rusty_sqlite::rusqlite::Connection;
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
            "rusty_meshed_ports_test_{}_{n}.db",
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

    // Path params come from Router::dispatch matching the pattern
    // against the *actual* request path -- it overwrites whatever
    // `params` a caller sets up front. So tests build a real path with
    // the real id interpolated in, exactly like a real client would,
    // rather than pre-seeding `params` on a placeholder path.
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

    fn create_product(state: &Arc<AppState>) -> i64 {
        let conn = state.get_session().unwrap();
        conn.execute(
            "INSERT INTO data_products (name, owner, version, domain, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["orders", "team-a", "1.0.0", "commerce", "Order events"],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[rusty_tokio::test]
    async fn create_input_port_returns_201_with_assigned_ids() {
        let state = temp_state();
        let product_id = create_product(&state);
        let mut body = Json::object();
        body.insert("topic_name", "upstream.topic");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/input-ports"),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(json.get("id").unwrap().as_f64().unwrap() > 0.0);
        assert_eq!(
            json.get("data_product_id").unwrap().as_f64(),
            Some(product_id as f64)
        );
        assert!(json.get("description").unwrap().is_null());
    }

    #[rusty_tokio::test]
    async fn create_input_port_returns_404_for_unknown_product() {
        let state = temp_state();
        let mut body = Json::object();
        body.insert("topic_name", "upstream.topic");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/data-products/999/input-ports".to_string(),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some("Data product 999 not found.")
        );
    }

    #[rusty_tokio::test]
    async fn create_input_port_stores_optional_description() {
        let state = temp_state();
        let product_id = create_product(&state);
        let mut body = Json::object();
        body.insert("topic_name", "upstream.topic");
        body.insert("description", "some notes");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/input-ports"),
                body,
            ))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("description").unwrap().as_str(),
            Some("some notes")
        );
    }

    #[rusty_tokio::test]
    async fn list_input_ports_returns_404_for_unknown_product() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Get,
                "/data-products/999/input-ports".to_string(),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn list_input_ports_scopes_to_the_given_product() {
        let state = temp_state();
        let product_a = create_product(&state);
        let product_b = create_product(&state);
        let r = router((*state).clone());

        let mut body = Json::object();
        body.insert("topic_name", "a.topic");
        r.dispatch(req(
            Method::Post,
            format!("/data-products/{product_a}/input-ports"),
            body,
        ))
        .await;
        let mut body = Json::object();
        body.insert("topic_name", "b.topic");
        r.dispatch(req(
            Method::Post,
            format!("/data-products/{product_b}/input-ports"),
            body,
        ))
        .await;

        let response = r
            .dispatch(req(
                Method::Get,
                format!("/data-products/{product_a}/input-ports"),
                Json::object(),
            ))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let ports = json.as_array().unwrap();
        assert_eq!(ports.len(), 1);
        assert_eq!(
            ports[0].get("topic_name").unwrap().as_str(),
            Some("a.topic")
        );
    }

    #[rusty_tokio::test]
    async fn delete_input_port_returns_204() {
        let state = temp_state();
        let product_id = create_product(&state);
        let r = router((*state).clone());
        let mut body = Json::object();
        body.insert("topic_name", "upstream.topic");
        let create_response = r
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/input-ports"),
                body,
            ))
            .await;
        let created = Json::parse(std::str::from_utf8(&create_response.body).unwrap()).unwrap();
        let port_id = created.get("id").unwrap().as_f64().unwrap() as i64;

        let response = r
            .dispatch(req(
                Method::Delete,
                format!("/data-products/{product_id}/input-ports/{port_id}"),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
    }

    #[rusty_tokio::test]
    async fn delete_input_port_returns_404_for_a_port_on_a_different_product() {
        let state = temp_state();
        let product_a = create_product(&state);
        let product_b = create_product(&state);
        let r = router((*state).clone());
        let mut body = Json::object();
        body.insert("topic_name", "a.topic");
        let create_response = r
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_a}/input-ports"),
                body,
            ))
            .await;
        let created = Json::parse(std::str::from_utf8(&create_response.body).unwrap()).unwrap();
        let port_id = created.get("id").unwrap().as_f64().unwrap() as i64;

        let response = r
            .dispatch(req(
                Method::Delete,
                format!("/data-products/{product_b}/input-ports/{port_id}"),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some(format!("Input port {port_id} not found on data product {product_b}.").as_str())
        );
    }

    #[rusty_tokio::test]
    async fn create_output_port_round_trips_event_type() {
        let state = temp_state();
        let product_id = create_product(&state);
        let mut body = Json::object();
        body.insert("topic_name", "downstream.topic");
        body.insert("schema_subject", "downstream.topic-value");
        body.insert("event_type", "state");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/output-ports"),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(json.get("event_type").unwrap().as_str(), Some("state"));
    }

    #[rusty_tokio::test]
    async fn create_output_port_returns_422_for_an_invalid_event_type() {
        let state = temp_state();
        let product_id = create_product(&state);
        let mut body = Json::object();
        body.insert("topic_name", "downstream.topic");
        body.insert("schema_subject", "downstream.topic-value");
        body.insert("event_type", "not-a-real-type");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/output-ports"),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[rusty_tokio::test]
    async fn create_output_port_returns_404_for_unknown_product() {
        let state = temp_state();
        let mut body = Json::object();
        body.insert("topic_name", "downstream.topic");
        body.insert("schema_subject", "downstream.topic-value");
        body.insert("event_type", "delta");
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/data-products/999/output-ports".to_string(),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn list_output_ports_scopes_to_the_given_product() {
        let state = temp_state();
        let product_a = create_product(&state);
        let product_b = create_product(&state);
        let r = router((*state).clone());

        let mut body = Json::object();
        body.insert("topic_name", "a.topic");
        body.insert("schema_subject", "a.topic-value");
        body.insert("event_type", "delta");
        r.dispatch(req(
            Method::Post,
            format!("/data-products/{product_a}/output-ports"),
            body,
        ))
        .await;
        let mut body = Json::object();
        body.insert("topic_name", "b.topic");
        body.insert("schema_subject", "b.topic-value");
        body.insert("event_type", "delta");
        r.dispatch(req(
            Method::Post,
            format!("/data-products/{product_b}/output-ports"),
            body,
        ))
        .await;

        let response = r
            .dispatch(req(
                Method::Get,
                format!("/data-products/{product_b}/output-ports"),
                Json::object(),
            ))
            .await;
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let ports = json.as_array().unwrap();
        assert_eq!(ports.len(), 1);
        assert_eq!(
            ports[0].get("topic_name").unwrap().as_str(),
            Some("b.topic")
        );
    }

    #[rusty_tokio::test]
    async fn delete_output_port_returns_204() {
        let state = temp_state();
        let product_id = create_product(&state);
        let r = router((*state).clone());
        let mut body = Json::object();
        body.insert("topic_name", "downstream.topic");
        body.insert("schema_subject", "downstream.topic-value");
        body.insert("event_type", "delta");
        let create_response = r
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/output-ports"),
                body,
            ))
            .await;
        let created = Json::parse(std::str::from_utf8(&create_response.body).unwrap()).unwrap();
        let port_id = created.get("id").unwrap().as_f64().unwrap() as i64;

        let response = r
            .dispatch(req(
                Method::Delete,
                format!("/data-products/{product_id}/output-ports/{port_id}"),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
    }

    #[rusty_tokio::test]
    async fn delete_output_port_returns_404_for_a_port_on_a_different_product() {
        let state = temp_state();
        let product_a = create_product(&state);
        let product_b = create_product(&state);
        let r = router((*state).clone());
        let mut body = Json::object();
        body.insert("topic_name", "a.topic");
        body.insert("schema_subject", "a.topic-value");
        body.insert("event_type", "delta");
        let create_response = r
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_a}/output-ports"),
                body,
            ))
            .await;
        let created = Json::parse(std::str::from_utf8(&create_response.body).unwrap()).unwrap();
        let port_id = created.get("id").unwrap().as_f64().unwrap() as i64;

        let response = r
            .dispatch(req(
                Method::Delete,
                format!("/data-products/{product_b}/output-ports/{port_id}"),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }
}
