//! Data contract CRUD endpoints -- the Rust port of
//! `meshed.registry.routers.contracts` (REG-076..086). Each output
//! port may have at most one data contract (enforced with a `UNIQUE`
//! FK at the DB schema level too, REG-023): a port without one is
//! ungoverned.

use super::detail_error;
use crate::app::AppState;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use crate::models::schemas::DataContractCreate;
use crate::models::DataContract;
use rusty_http::StatusCode;
use rusty_request::Json;
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

fn contract_not_found(port_id: i64) -> Response {
    detail_error(
        StatusCode::NOT_FOUND,
        format!("No data contract found for output port {port_id}."),
    )
}

fn product_exists(conn: &Connection, product_id: i64) -> rusty_sqlite::rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM data_products WHERE id = ?1)",
        params![product_id],
        |row| row.get(0),
    )
}

fn port_belongs_to_product(
    conn: &Connection,
    product_id: i64,
    port_id: i64,
) -> rusty_sqlite::rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM output_ports WHERE id = ?1 AND data_product_id = ?2)",
        params![port_id, product_id],
        |row| row.get(0),
    )
}

fn fetch_contract(
    conn: &Connection,
    port_id: i64,
) -> rusty_sqlite::rusqlite::Result<Option<DataContract>> {
    conn.query_row(
        "SELECT id, output_port_id, schema_ref, owner, slo_freshness_seconds, slo_completeness_pct, quality_assertions \
         FROM data_contracts WHERE output_port_id = ?1",
        params![port_id],
        |row| {
            Ok(DataContract {
                id: row.get(0)?,
                output_port_id: row.get(1)?,
                schema_ref: row.get(2)?,
                owner: row.get(3)?,
                slo_freshness_seconds: row.get(4)?,
                slo_completeness_pct: row.get(5)?,
                quality_assertions: row.get(6)?,
            })
        },
    )
    .optional()
}

/// Decodes `quality_assertions` from its JSON-encoded storage form back
/// to a JSON array of strings (REG-081), mirroring
/// `DataContractPublic::from_row`'s decoding at the JSON-response
/// layer instead of the Rust-struct layer.
fn to_public_json(contract: &DataContract) -> Json {
    let assertions: Vec<String> =
        rusty_json::from_str(&contract.quality_assertions).unwrap_or_default();
    let mut assertions_array = Json::array();
    for assertion in &assertions {
        assertions_array.push(assertion.as_str());
    }

    let mut json = Json::object();
    json.insert("id", contract.id);
    json.insert("output_port_id", contract.output_port_id);
    json.insert("schema_ref", contract.schema_ref.as_str());
    json.insert("owner", contract.owner.as_str());
    json.insert("slo_freshness_seconds", contract.slo_freshness_seconds);
    json.insert("slo_completeness_pct", contract.slo_completeness_pct);
    json.insert("quality_assertions", assertions_array);
    json
}

/// Runs the shared product-exists / port-belongs-to-product guard
/// every contract route needs before touching the contract itself.
/// `Ok(())` on success; `Err(response)` is the 404 (or 500) to return
/// immediately.
fn guard_product_and_port(
    conn: &Connection,
    product_id: i64,
    port_id: i64,
) -> Result<(), Response> {
    match product_exists(conn, product_id) {
        Ok(true) => {}
        Ok(false) => return Err(product_not_found(product_id)),
        Err(_) => return Err(internal_error()),
    }
    match port_belongs_to_product(conn, product_id, port_id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(port_not_found(port_id, product_id)),
        Err(_) => Err(internal_error()),
    }
}

async fn create(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Some(port_id) = parse_id(&req, "port_id") else {
        return bad_id("port_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    if let Err(response) = guard_product_and_port(&conn, product_id, port_id) {
        return response;
    }

    // REG-079: one-contract-per-port invariant, enforced here (409) in
    // addition to the DB's own UNIQUE constraint (REG-023).
    match fetch_contract(&conn, port_id) {
        Ok(Some(_)) => {
            return detail_error(
                StatusCode::CONFLICT,
                format!("Output port {port_id} already has a registered data contract."),
            )
        }
        Ok(None) => {}
        Err(_) => return internal_error(),
    }

    let Ok(body) = req.json() else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid JSON body");
    };
    let schema_ref = body.get("schema_ref").and_then(|v| v.as_str());
    let owner = body.get("owner").and_then(|v| v.as_str());
    let slo_freshness_seconds = body.get("slo_freshness_seconds").and_then(|v| v.as_f64());
    let slo_completeness_pct = body.get("slo_completeness_pct").and_then(|v| v.as_f64());
    let quality_assertions = body.get("quality_assertions").and_then(|v| v.as_array());

    let (
        Some(schema_ref),
        Some(owner),
        Some(slo_freshness_seconds),
        Some(slo_completeness_pct),
        Some(quality_assertions),
    ) = (
        schema_ref,
        owner,
        slo_freshness_seconds,
        slo_completeness_pct,
        quality_assertions,
    )
    else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "schema_ref, owner, slo_freshness_seconds, slo_completeness_pct, and quality_assertions are all required",
        );
    };
    let quality_assertions: Option<Vec<String>> = quality_assertions
        .iter()
        .map(|value| value.as_str().map(str::to_string))
        .collect();
    let Some(quality_assertions) = quality_assertions else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "quality_assertions must be an array of strings",
        );
    };

    let create = match DataContractCreate::new(
        schema_ref,
        owner,
        slo_freshness_seconds as i64,
        slo_completeness_pct,
        quality_assertions,
    ) {
        Ok(create) => create,
        Err(err) => return detail_error(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()),
    };

    // REG-080: stored as a JSON-encoded string.
    let quality_json =
        rusty_json::to_string(&create.quality_assertions).unwrap_or_else(|_| "[]".to_string());
    let inserted = conn.execute(
        "INSERT INTO data_contracts (output_port_id, schema_ref, owner, slo_freshness_seconds, slo_completeness_pct, quality_assertions) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            port_id,
            create.schema_ref,
            create.owner,
            create.slo_freshness_seconds,
            create.slo_completeness_pct,
            quality_json,
        ],
    );
    if inserted.is_err() {
        return internal_error();
    }

    let contract = DataContract {
        id: conn.last_insert_rowid(),
        output_port_id: port_id,
        schema_ref: create.schema_ref,
        owner: create.owner,
        slo_freshness_seconds: create.slo_freshness_seconds,
        slo_completeness_pct: create.slo_completeness_pct,
        quality_assertions: quality_json,
    };
    Response::json(StatusCode::CREATED, &to_public_json(&contract))
}

async fn get(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Some(port_id) = parse_id(&req, "port_id") else {
        return bad_id("port_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    if let Err(response) = guard_product_and_port(&conn, product_id, port_id) {
        return response;
    }

    match fetch_contract(&conn, port_id) {
        Ok(Some(contract)) => Response::json(StatusCode::OK, &to_public_json(&contract)),
        Ok(None) => contract_not_found(port_id),
        Err(_) => internal_error(),
    }
}

async fn delete(state: Arc<AppState>, req: Request) -> Response {
    let Some(product_id) = parse_id(&req, "product_id") else {
        return bad_id("product_id");
    };
    let Some(port_id) = parse_id(&req, "port_id") else {
        return bad_id("port_id");
    };
    let Ok(conn) = state.get_session() else {
        return session_error();
    };
    if let Err(response) = guard_product_and_port(&conn, product_id, port_id) {
        return response;
    }

    match fetch_contract(&conn, port_id) {
        Ok(Some(_)) => {}
        Ok(None) => return contract_not_found(port_id),
        Err(_) => return internal_error(),
    }

    match conn.execute(
        "DELETE FROM data_contracts WHERE output_port_id = ?1",
        params![port_id],
    ) {
        Ok(_) => Response::new(StatusCode::NO_CONTENT),
        Err(_) => internal_error(),
    }
}

/// Builds the `/data-products/{product_id}/output-ports/{port_id}/contract`
/// router, bound to `state` for DB access.
pub fn router(state: Arc<AppState>) -> Router {
    let s = state.clone();
    let router = Router::new().post(
        "/data-products/{product_id}/output-ports/{port_id}/contract",
        move |req| {
            let state = s.clone();
            async move { create(state, req).await }
        },
    );

    let s = state.clone();
    let router = router.get(
        "/data-products/{product_id}/output-ports/{port_id}/contract",
        move |req| {
            let state = s.clone();
            async move { get(state, req).await }
        },
    );

    router.delete(
        "/data-products/{product_id}/output-ports/{port_id}/contract",
        move |req| {
            let state = state.clone();
            async move { delete(state, req).await }
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
            "rusty_meshed_contracts_test_{}_{n}.db",
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

    fn valid_contract_body() -> Json {
        let mut body = Json::object();
        body.insert("schema_ref", "orders.created-value:1");
        body.insert("owner", "team-a");
        body.insert("slo_freshness_seconds", 60);
        body.insert("slo_completeness_pct", 99.5);
        let mut assertions = Json::array();
        assertions.push("no nulls in order_id");
        body.insert("quality_assertions", assertions);
        body
    }

    #[rusty_tokio::test]
    async fn create_returns_201_when_product_and_port_exist_with_no_contract() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/output-ports/{port_id}/contract"),
                valid_contract_body(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(json.get("id").unwrap().as_f64().unwrap() > 0.0);
        assert_eq!(
            json.get("output_port_id").unwrap().as_f64(),
            Some(port_id as f64)
        );
        assert_eq!(
            json.get("quality_assertions")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[rusty_tokio::test]
    async fn create_returns_404_when_product_missing() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                "/data-products/999/output-ports/1/contract".to_string(),
                valid_contract_body(),
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
    async fn create_returns_404_when_port_missing_or_mismatched() {
        let state = temp_state();
        let (product_id, _port_id) = create_product_and_port(&state);
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/output-ports/999/contract"),
                valid_contract_body(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some(format!("Output port 999 not found on data product {product_id}.").as_str())
        );
    }

    #[rusty_tokio::test]
    async fn create_returns_409_when_contract_already_exists() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let r = router((*state).clone());
        let path = format!("/data-products/{product_id}/output-ports/{port_id}/contract");
        r.dispatch(req(Method::Post, path.clone(), valid_contract_body()))
            .await;
        let response = r
            .dispatch(req(Method::Post, path, valid_contract_body()))
            .await;
        assert_eq!(response.status, StatusCode::CONFLICT);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some(format!("Output port {port_id} already has a registered data contract.").as_str())
        );
    }

    #[rusty_tokio::test]
    async fn create_returns_422_when_quality_assertions_is_empty() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let mut body = valid_contract_body();
        body.insert("quality_assertions", Json::array());
        let response = router((*state).clone())
            .dispatch(req(
                Method::Post,
                format!("/data-products/{product_id}/output-ports/{port_id}/contract"),
                body,
            ))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[rusty_tokio::test]
    async fn get_returns_the_contract_with_decoded_quality_assertions() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let r = router((*state).clone());
        let path = format!("/data-products/{product_id}/output-ports/{port_id}/contract");
        r.dispatch(req(Method::Post, path.clone(), valid_contract_body()))
            .await;

        let response = r.dispatch(req(Method::Get, path, Json::object())).await;
        assert_eq!(response.status, StatusCode::OK);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let assertions = json.get("quality_assertions").unwrap().as_array().unwrap();
        assert_eq!(assertions[0].as_str(), Some("no nulls in order_id"));
    }

    #[rusty_tokio::test]
    async fn get_returns_404_when_no_contract_exists() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let response = router((*state).clone())
            .dispatch(req(
                Method::Get,
                format!("/data-products/{product_id}/output-ports/{port_id}/contract"),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let json = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            json.get("detail").unwrap().as_str(),
            Some(format!("No data contract found for output port {port_id}.").as_str())
        );
    }

    #[rusty_tokio::test]
    async fn get_returns_404_when_product_or_port_missing() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Get,
                "/data-products/999/output-ports/1/contract".to_string(),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn delete_returns_204_and_removes_the_contract() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let r = router((*state).clone());
        let path = format!("/data-products/{product_id}/output-ports/{port_id}/contract");
        r.dispatch(req(Method::Post, path.clone(), valid_contract_body()))
            .await;

        let response = r
            .dispatch(req(Method::Delete, path.clone(), Json::object()))
            .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);

        let get_response = r.dispatch(req(Method::Get, path, Json::object())).await;
        assert_eq!(get_response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn delete_returns_404_when_no_contract_exists() {
        let state = temp_state();
        let (product_id, port_id) = create_product_and_port(&state);
        let response = router((*state).clone())
            .dispatch(req(
                Method::Delete,
                format!("/data-products/{product_id}/output-ports/{port_id}/contract"),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn delete_returns_404_when_product_or_port_missing() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(
                Method::Delete,
                "/data-products/999/output-ports/1/contract".to_string(),
                Json::object(),
            ))
            .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }
}
