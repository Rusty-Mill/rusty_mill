//! Governance ad-hoc evaluation router -- the Rust port of
//! `meshed.registry.routers.governance` (REG-104..106). A dry-run: it
//! evaluates a data-product payload against the same shared
//! [`default_governance_engine`] singleton `POST /data-products` uses,
//! but never persists anything and always returns 200, even when the
//! payload fails every policy.

use super::detail_error;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::http::router::Router;
use crate::models::schemas::{default_governance_engine, DataProductCreate};
use crate::models::MaturityTier;
use rusty_http::StatusCode;
use rusty_request::Json;

async fn evaluate(req: Request) -> Response {
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

    // REG-104: always 200, even on failure -- a non-destructive dry-run.
    let violations = default_governance_engine().evaluate(&create);
    let mut violations_array = Json::array();
    for violation in &violations {
        violations_array.push(violation.as_str());
    }
    let mut body = Json::object();
    body.insert("violations", violations_array);
    body.insert("passed", violations.is_empty());
    Response::json(StatusCode::OK, &body)
}

/// Builds the `/governance` router. Needs no DB access -- evaluation
/// never persists (REG-106).
pub fn router() -> Router {
    Router::new().post(
        "/governance/evaluate",
        |req| async move { evaluate(req).await },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::{HeaderMap, Method};

    fn req(body: Json) -> Request {
        Request {
            method: Method::Post,
            path: "/governance/evaluate".to_string(),
            query: Vec::new(),
            params: Vec::new(),
            headers: HeaderMap::new(),
            body: body.to_json_string().into_bytes(),
        }
    }

    fn valid_body() -> Json {
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
    async fn returns_200_and_passed_true_for_a_valid_payload() {
        let response = router().dispatch(req(valid_body())).await;
        assert_eq!(response.status, StatusCode::OK);
        let body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(body.get("passed").unwrap().as_bool(), Some(true));
        assert!(body
            .get("violations")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[rusty_tokio::test]
    async fn returns_200_with_violations_for_a_failing_payload_not_422() {
        let mut body = valid_body();
        body.insert("description", "short");
        body.insert("version", "not-semver");
        body.insert("domain", "NotLowercase");
        let response = router().dispatch(req(body)).await;
        // REG-104: still 200 even though every policy fails.
        assert_eq!(response.status, StatusCode::OK);
        let response_body = Json::parse(std::str::from_utf8(&response.body).unwrap()).unwrap();
        assert_eq!(response_body.get("passed").unwrap().as_bool(), Some(false));
        assert_eq!(
            response_body
                .get("violations")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[rusty_tokio::test]
    async fn does_not_persist_anything() {
        // No AppState/DB wiring exists in this router at all -- there's
        // nothing it could persist to, which is the point (REG-106).
        let response = router().dispatch(req(valid_body())).await;
        assert_eq!(response.status, StatusCode::OK);
    }
}
