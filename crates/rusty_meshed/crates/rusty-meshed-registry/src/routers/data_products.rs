//! Data products CRUD and discovery router -- the Rust port of
//! `meshed.registry.routers.data_products` (REG-034..058).
//!
//! Discovery filters (`domain`/`owner`/`tag`/`event_type`) are additive
//! (AND-combined, REG-048) and built into one dynamic SQL query per
//! request rather than composing a query-builder AST (there's no
//! SQLAlchemy-equivalent in this workspace to build one on top of, and
//! four optional `WHERE`/`JOIN` fragments don't need one).

use super::{detail_error, not_found};
use crate::app::AppState;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use crate::models::schemas::{default_governance_engine, DataProductCreate};
use crate::models::{DataProduct, MaturityTier};
use rusty_http::StatusCode;
use rusty_meshed_core::EventType;
use rusty_request::Json;
use rusty_sqlite::rusqlite::types::ToSql;
use rusty_sqlite::rusqlite::{params, Connection, OptionalExtension, Row};
use std::sync::Arc;

const RESOURCE: &str = "Data product";

fn to_json(product: &DataProduct) -> Json {
    let mut json = Json::object();
    json.insert("id", product.id);
    json.insert("name", product.name.as_str());
    json.insert("owner", product.owner.as_str());
    json.insert("version", product.version.as_str());
    json.insert("domain", product.domain.as_str());
    json.insert("description", product.description.as_str());
    json.insert("maturity_tier", product.maturity_tier.as_str());
    json.insert("tags", product.tags.as_str());
    json
}

fn row_to_product(row: &Row) -> rusty_sqlite::rusqlite::Result<DataProduct> {
    let maturity_tier: String = row.get(6)?;
    Ok(DataProduct {
        id: row.get(0)?,
        name: row.get(1)?,
        owner: row.get(2)?,
        version: row.get(3)?,
        domain: row.get(4)?,
        description: row.get(5)?,
        maturity_tier: MaturityTier::parse(&maturity_tier).unwrap_or_default(),
        tags: row.get(7)?,
    })
}

fn fetch_by_id(conn: &Connection, id: i64) -> rusty_sqlite::rusqlite::Result<Option<DataProduct>> {
    conn.query_row(
        "SELECT id, name, owner, version, domain, description, maturity_tier, tags \
         FROM data_products WHERE id = ?1",
        params![id],
        row_to_product,
    )
    .optional()
}

fn parse_id(req: &Request) -> Option<i64> {
    req.param("id").and_then(|value| value.parse().ok())
}

fn internal_error() -> Response {
    detail_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

async fn create(state: Arc<AppState>, req: Request) -> Response {
    let Ok(body) = req.json() else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid JSON body");
    };
    let field = |name: &str| body.get(name).and_then(|value| value.as_str());

    let (Some(name), Some(owner), Some(version), Some(domain), Some(description)) = (
        field("name"),
        field("owner"),
        field("version"),
        field("domain"),
        field("description"),
    ) else {
        return detail_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "name, owner, version, domain, and description are all required",
        );
    };

    let mut create = DataProductCreate::new(name, owner, version, domain, description);
    if let Some(tier) = field("maturity_tier") {
        match MaturityTier::parse(tier) {
            Some(tier) => create = create.with_maturity_tier(tier),
            None => {
                return detail_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("'{tier}' is not a valid maturity_tier"),
                )
            }
        }
    }
    if let Some(tags) = field("tags") {
        create = create.with_tags(tags);
    }

    // REG-035/043: the shared _DEFAULT_ENGINE singleton gates creation.
    let violations = default_governance_engine().evaluate(&create);
    if !violations.is_empty() {
        let mut violations_array = Json::array();
        for violation in &violations {
            violations_array.push(violation.as_str());
        }
        let mut detail = Json::object();
        detail.insert("governance_violations", violations_array);
        let mut body = Json::object();
        body.insert("detail", detail);
        return Response::json(StatusCode::UNPROCESSABLE_ENTITY, &body);
    }

    let Ok(conn) = state.get_session() else {
        return detail_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database engine is not initialized",
        );
    };
    // REG-057: no uniqueness check on name -- duplicates are permitted.
    let inserted = conn.execute(
        "INSERT INTO data_products (name, owner, version, domain, description, maturity_tier, tags) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            create.name,
            create.owner,
            create.version,
            create.domain,
            create.description,
            create.maturity_tier.as_str(),
            create.tags,
        ],
    );
    if inserted.is_err() {
        return internal_error();
    }
    let product = DataProduct {
        id: conn.last_insert_rowid(),
        name: create.name,
        owner: create.owner,
        version: create.version,
        domain: create.domain,
        description: create.description,
        maturity_tier: create.maturity_tier,
        tags: create.tags,
    };
    Response::json(StatusCode::CREATED, &to_json(&product))
}

async fn list(state: Arc<AppState>, req: Request) -> Response {
    let Ok(conn) = state.get_session() else {
        return detail_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database engine is not initialized",
        );
    };

    let event_type = req.query_param("event_type");
    if let Some(event_type) = event_type {
        if EventType::parse(event_type).is_none() {
            return detail_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("'{event_type}' is not a valid event_type"),
            );
        }
    }

    let offset: i64 = match req.query_param("offset").map(str::parse) {
        Some(Ok(value)) => value,
        Some(Err(_)) => {
            return detail_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "offset must be an integer",
            )
        }
        None => 0,
    };
    if offset < 0 {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "offset must be >= 0");
    }
    let limit: i64 = match req.query_param("limit").map(str::parse) {
        Some(Ok(value)) => value,
        Some(Err(_)) => {
            return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "limit must be an integer")
        }
        None => 100,
    };
    if limit > 100 {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "limit must be <= 100");
    }

    // REG-044..048: every filter is optional and AND-combined.
    let mut select = "SELECT dp.id, dp.name, dp.owner, dp.version, dp.domain, dp.description, dp.maturity_tier, dp.tags FROM data_products dp".to_string();
    if event_type.is_some() {
        select = select.replacen("SELECT ", "SELECT DISTINCT ", 1);
        select.push_str(" LEFT JOIN output_ports op ON op.data_product_id = dp.id");
    }

    let mut conditions: Vec<String> = Vec::new();
    let mut bindings: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(domain) = req.query_param("domain") {
        conditions.push("dp.domain = ?".to_string());
        bindings.push(Box::new(domain.to_string()));
    }
    if let Some(owner) = req.query_param("owner") {
        conditions.push("dp.owner = ?".to_string());
        bindings.push(Box::new(owner.to_string()));
    }
    if let Some(tag) = req.query_param("tag") {
        conditions.push("dp.tags LIKE ?".to_string());
        bindings.push(Box::new(format!("%{tag}%")));
    }
    if let Some(event_type) = event_type {
        conditions.push("op.event_type = ?".to_string());
        bindings.push(Box::new(event_type.to_string()));
    }
    if !conditions.is_empty() {
        select.push_str(" WHERE ");
        select.push_str(&conditions.join(" AND "));
    }
    select.push_str(" LIMIT ? OFFSET ?");
    bindings.push(Box::new(limit));
    bindings.push(Box::new(offset));

    let Ok(mut stmt) = conn.prepare(&select) else {
        return internal_error();
    };
    let param_refs: Vec<&dyn ToSql> = bindings.iter().map(AsRef::as_ref).collect();
    let Ok(rows) = stmt.query_map(param_refs.as_slice(), row_to_product) else {
        return internal_error();
    };

    let mut products = Json::array();
    for row in rows {
        match row {
            Ok(product) => {
                products.push(to_json(&product));
            }
            Err(_) => return internal_error(),
        }
    }
    Response::json(StatusCode::OK, &products)
}

async fn get(state: Arc<AppState>, req: Request) -> Response {
    let Some(id) = parse_id(&req) else {
        return not_found(RESOURCE);
    };
    let Ok(conn) = state.get_session() else {
        return detail_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database engine is not initialized",
        );
    };
    match fetch_by_id(&conn, id) {
        Ok(Some(product)) => Response::json(StatusCode::OK, &to_json(&product)),
        Ok(None) => not_found(RESOURCE),
        Err(_) => internal_error(),
    }
}

async fn update(state: Arc<AppState>, req: Request) -> Response {
    let Some(id) = parse_id(&req) else {
        return not_found(RESOURCE);
    };
    let Ok(conn) = state.get_session() else {
        return detail_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database engine is not initialized",
        );
    };
    let existing = match fetch_by_id(&conn, id) {
        Ok(Some(product)) => product,
        Ok(None) => return not_found(RESOURCE),
        Err(_) => return internal_error(),
    };

    let Ok(body) = req.json() else {
        return detail_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid JSON body");
    };
    let mut product = existing;
    if let Some(value) = body.get("name").and_then(|v| v.as_str()) {
        product.name = value.to_string();
    }
    if let Some(value) = body.get("owner").and_then(|v| v.as_str()) {
        product.owner = value.to_string();
    }
    if let Some(value) = body.get("version").and_then(|v| v.as_str()) {
        product.version = value.to_string();
    }
    if let Some(value) = body.get("domain").and_then(|v| v.as_str()) {
        product.domain = value.to_string();
    }
    if let Some(value) = body.get("description").and_then(|v| v.as_str()) {
        product.description = value.to_string();
    }
    if let Some(value) = body.get("tags").and_then(|v| v.as_str()) {
        product.tags = value.to_string();
    }
    if let Some(value) = body.get("maturity_tier").and_then(|v| v.as_str()) {
        match MaturityTier::parse(value) {
            Some(tier) => product.maturity_tier = tier,
            None => {
                return detail_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("'{value}' is not a valid maturity_tier"),
                )
            }
        }
    }

    // REG-054: PATCH never re-runs governance -- only POST does.
    let updated = conn.execute(
        "UPDATE data_products SET name = ?1, owner = ?2, version = ?3, domain = ?4, \
         description = ?5, maturity_tier = ?6, tags = ?7 WHERE id = ?8",
        params![
            product.name,
            product.owner,
            product.version,
            product.domain,
            product.description,
            product.maturity_tier.as_str(),
            product.tags,
            id,
        ],
    );
    if updated.is_err() {
        return internal_error();
    }
    Response::json(StatusCode::OK, &to_json(&product))
}

async fn delete(state: Arc<AppState>, req: Request) -> Response {
    let Some(id) = parse_id(&req) else {
        return not_found(RESOURCE);
    };
    let Ok(conn) = state.get_session() else {
        return detail_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database engine is not initialized",
        );
    };
    match conn.execute("DELETE FROM data_products WHERE id = ?1", params![id]) {
        Ok(0) => not_found(RESOURCE),
        Ok(_) => Response::new(StatusCode::NO_CONTENT),
        Err(_) => internal_error(),
    }
}

/// Builds the `/data-products` router, bound to `state` for DB access.
pub fn router(state: Arc<AppState>) -> Router {
    let s = state.clone();
    let router = Router::new().post("/data-products", move |req| {
        let state = s.clone();
        async move { create(state, req).await }
    });

    let s = state.clone();
    let router = router.get("/data-products", move |req| {
        let state = s.clone();
        async move { list(state, req).await }
    });

    let s = state.clone();
    let router = router.get("/data-products/{id}", move |req| {
        let state = s.clone();
        async move { get(state, req).await }
    });

    let s = state.clone();
    let router = router.patch("/data-products/{id}", move |req| {
        let state = s.clone();
        async move { update(state, req).await }
    });

    router.delete("/data-products/{id}", move |req| {
        let state = state.clone();
        async move { delete(state, req).await }
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
            "rusty_meshed_data_products_test_{}_{n}.db",
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

    fn req(method: Method, path: &str, body: Json) -> HttpRequest {
        HttpRequest {
            method,
            path: path.to_string(),
            query: Vec::new(),
            params: Vec::new(),
            headers: HeaderMap::new(),
            body: body.to_json_string().into_bytes(),
        }
    }

    fn valid_product_body() -> Json {
        let mut body = Json::object();
        body.insert("name", "orders");
        body.insert("owner", "team-commerce");
        body.insert("version", "1.0.0");
        body.insert("domain", "commerce");
        body.insert(
            "description",
            "Order lifecycle events for the commerce domain.",
        );
        body
    }

    #[rusty_tokio::test]
    async fn create_returns_201_with_assigned_id_and_default_maturity_tier() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(Method::Post, "/data-products", valid_product_body()))
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(body.get("id").unwrap().as_f64().unwrap() > 0.0);
        assert_eq!(body.get("maturity_tier").unwrap().as_str(), Some("mvp"));
        assert_eq!(body.get("tags").unwrap().as_str(), Some("[]"));
    }

    #[rusty_tokio::test]
    async fn create_returns_422_on_governance_violation() {
        let state = temp_state();
        let mut body = valid_product_body();
        body.insert("description", "short");
        let response = router((*state).clone())
            .dispatch(req(Method::Post, "/data-products", body))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let violations = body
            .get("detail")
            .unwrap()
            .get("governance_violations")
            .unwrap();
        assert!(!violations.as_array().unwrap().is_empty());
    }

    #[rusty_tokio::test]
    async fn create_returns_422_when_a_required_field_is_missing() {
        let state = temp_state();
        // No "owner" field -- name/version/domain/description alone
        // aren't enough (REG-058).
        let mut incomplete = Json::object();
        incomplete.insert("name", "orders");
        incomplete.insert("version", "1.0.0");
        incomplete.insert("domain", "commerce");
        incomplete.insert(
            "description",
            "Order lifecycle events for the commerce domain.",
        );
        let response = router((*state).clone())
            .dispatch(req(Method::Post, "/data-products", incomplete))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[rusty_tokio::test]
    async fn create_permits_duplicate_names() {
        let state = temp_state();
        let r = router((*state).clone());
        let first = r
            .dispatch(req(Method::Post, "/data-products", valid_product_body()))
            .await;
        let second = r
            .dispatch(req(Method::Post, "/data-products", valid_product_body()))
            .await;
        assert_eq!(first.status, StatusCode::CREATED);
        assert_eq!(second.status, StatusCode::CREATED);
    }

    async fn create_one(state: &Arc<AppState>) -> i64 {
        let response = router(state.clone())
            .dispatch(req(Method::Post, "/data-products", valid_product_body()))
            .await;
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        body.get("id").unwrap().as_f64().unwrap() as i64
    }

    #[rusty_tokio::test]
    async fn get_returns_404_for_an_unknown_id() {
        let state = temp_state();
        let mut request = req(Method::Get, "/data-products/999", Json::object());
        request.params.push(("id".to_string(), "999".to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(
            body.get("detail").unwrap().as_str(),
            Some("Data product not found")
        );
    }

    #[rusty_tokio::test]
    async fn get_returns_the_created_product() {
        let state = temp_state();
        let id = create_one(&state).await;
        let mut request = req(Method::Get, &format!("/data-products/{id}"), Json::object());
        request.params.push(("id".to_string(), id.to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(body.get("name").unwrap().as_str(), Some("orders"));
    }

    #[rusty_tokio::test]
    async fn patch_applies_only_provided_fields_and_skips_governance() {
        let state = temp_state();
        let id = create_one(&state).await;
        let mut patch_body = Json::object();
        // Governance would reject this description on create; PATCH must
        // still accept it (REG-054: PATCH never re-runs governance).
        patch_body.insert("description", "short");
        let mut request = req(Method::Patch, &format!("/data-products/{id}"), patch_body);
        request.params.push(("id".to_string(), id.to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(body.get("description").unwrap().as_str(), Some("short"));
        // Untouched fields survive the partial update.
        assert_eq!(body.get("name").unwrap().as_str(), Some("orders"));
    }

    #[rusty_tokio::test]
    async fn patch_returns_404_for_an_unknown_id() {
        let state = temp_state();
        let mut patch_body = Json::object();
        patch_body.insert("name", "new-name");
        let mut request = req(Method::Patch, "/data-products/999", patch_body);
        request.params.push(("id".to_string(), "999".to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn delete_returns_204_and_removes_the_product() {
        let state = temp_state();
        let id = create_one(&state).await;
        let mut request = req(
            Method::Delete,
            &format!("/data-products/{id}"),
            Json::object(),
        );
        request.params.push(("id".to_string(), id.to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);

        let mut get_request = req(Method::Get, &format!("/data-products/{id}"), Json::object());
        get_request.params.push(("id".to_string(), id.to_string()));
        let get_response = router((*state).clone()).dispatch(get_request).await;
        assert_eq!(get_response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn delete_cascades_to_input_ports() {
        let state = temp_state();
        let id = create_one(&state).await;
        {
            let conn = state.get_session().unwrap();
            conn.execute(
                "INSERT INTO input_ports (data_product_id, topic_name) VALUES (?1, ?2)",
                params![id, "upstream.topic"],
            )
            .unwrap();
        }
        let mut request = req(
            Method::Delete,
            &format!("/data-products/{id}"),
            Json::object(),
        );
        request.params.push(("id".to_string(), id.to_string()));
        router((*state).clone()).dispatch(request).await;

        let conn = state.get_session().unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM input_ports", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[rusty_tokio::test]
    async fn delete_returns_404_for_an_unknown_id() {
        let state = temp_state();
        let mut request = req(Method::Delete, "/data-products/999", Json::object());
        request.params.push(("id".to_string(), "999".to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn list_filters_by_domain_and_owner_additively() {
        let state = temp_state();
        let r = router((*state).clone());
        r.dispatch(req(Method::Post, "/data-products", valid_product_body()))
            .await;
        let mut other = valid_product_body();
        other.insert("domain", "finance");
        r.dispatch(req(Method::Post, "/data-products", other)).await;

        let mut request = req(Method::Get, "/data-products", Json::object());
        request
            .query
            .push(("domain".to_string(), "commerce".to_string()));
        request
            .query
            .push(("owner".to_string(), "team-commerce".to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        let products = body.as_array().unwrap();
        assert_eq!(products.len(), 1);
        assert_eq!(
            products[0].get("domain").unwrap().as_str(),
            Some("commerce")
        );
    }

    #[rusty_tokio::test]
    async fn list_filters_by_tag_substring() {
        let state = temp_state();
        let r = router((*state).clone());
        let mut tagged = valid_product_body();
        tagged.insert("tags", r#"["finance","audit"]"#);
        r.dispatch(req(Method::Post, "/data-products", tagged))
            .await;
        r.dispatch(req(Method::Post, "/data-products", valid_product_body()))
            .await;

        let mut request = req(Method::Get, "/data-products", Json::object());
        request.query.push(("tag".to_string(), "audit".to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(body.as_array().unwrap().len(), 1);
    }

    #[rusty_tokio::test]
    async fn list_offset_rejects_negative_values() {
        let state = temp_state();
        let mut request = req(Method::Get, "/data-products", Json::object());
        request.query.push(("offset".to_string(), "-1".to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[rusty_tokio::test]
    async fn list_limit_rejects_values_above_100() {
        let state = temp_state();
        let mut request = req(Method::Get, "/data-products", Json::object());
        request.query.push(("limit".to_string(), "101".to_string()));
        let response = router((*state).clone()).dispatch(request).await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[rusty_tokio::test]
    async fn list_is_empty_with_no_products() {
        let state = temp_state();
        let response = router((*state).clone())
            .dispatch(req(Method::Get, "/data-products", Json::object()))
            .await;
        assert_eq!(response.status, StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert!(body.as_array().unwrap().is_empty());
    }
}
