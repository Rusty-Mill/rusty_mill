use rp_router::RouterError;
use rusty_axum::{IntoResponse, Json, Response, StatusCode};
use serde_json::json;

pub fn router_error_response(err: RouterError) -> Response {
    let status = err.status_code();
    let retry_after_secs = err.retry_after_secs();
    json_error_with_retry_after(status, &err.to_string(), retry_after_secs)
}

pub fn json_error(status: u16, message: &str) -> Response {
    json_error_with_retry_after(status, message, None)
}

pub fn json_error_with_retry_after(
    status: u16,
    message: &str,
    retry_after_secs: Option<u64>,
) -> Response {
    // The range `http::StatusCode::from_u16` accepted; `rusty_http`'s
    // wraps any value, so the fallback is checked here.
    let status = if (100..1000).contains(&status) {
        StatusCode::from_u16(status)
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let resp = (
        status,
        Json(json!({ "error": { "message": message, "code": status.as_u16() } })),
    )
        .into_response();
    match retry_after_secs {
        Some(secs) => resp.with_header("retry-after", &secs.to_string()),
        None => resp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_core::ProviderError;

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = resp.body.collect().await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn json_error_sets_status_and_omits_retry_after() {
        let resp = json_error(401, "missing or invalid API key");
        assert_eq!(resp.status.as_u16(), 401);
        assert!(resp.headers.get("retry-after").is_none());
    }

    #[tokio::test]
    async fn json_error_body_carries_the_message_and_matching_code() {
        let resp = json_error(400, "bad request");
        let body = body_json(resp).await;
        assert_eq!(body["error"]["message"], "bad request");
        assert_eq!(body["error"]["code"], 400);
    }

    #[test]
    fn json_error_with_retry_after_sets_the_header() {
        let resp = json_error_with_retry_after(429, "rate limited", Some(5));
        assert_eq!(resp.status.as_u16(), 429);
        assert_eq!(resp.headers.get("retry-after"), Some("5"));
    }

    #[test]
    fn json_error_with_retry_after_none_omits_the_header() {
        let resp = json_error_with_retry_after(500, "oops", None);
        assert!(resp.headers.get("retry-after").is_none());
    }

    #[test]
    fn json_error_with_retry_after_falls_back_to_500_for_an_invalid_status_code() {
        let resp = json_error_with_retry_after(9999, "unmappable", None);
        assert_eq!(resp.status.as_u16(), 500);
    }

    #[tokio::test]
    async fn json_error_body_code_reflects_the_fallback_status_not_the_invalid_input() {
        let resp = json_error_with_retry_after(9999, "unmappable", None);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], 500);
    }

    #[test]
    fn router_error_response_delegates_status_and_retry_after_from_router_error() {
        let err = RouterError::from(ProviderError::RateLimited {
            retry_after_secs: Some(7),
        });
        let resp = router_error_response(err);
        assert_eq!(resp.status.as_u16(), 429);
        assert_eq!(resp.headers.get("retry-after"), Some("7"));
    }

    #[test]
    fn router_error_response_has_no_retry_after_for_a_non_rate_limited_error() {
        let resp = router_error_response(RouterError::InvalidModel("bogus".to_string()));
        assert_eq!(resp.status.as_u16(), 400);
        assert!(resp.headers.get("retry-after").is_none());
    }

    #[tokio::test]
    async fn router_error_response_body_message_matches_the_error_display() {
        let err = RouterError::ProviderNotConfigured("openai".to_string());
        let expected_message = err.to_string();
        let resp = router_error_response(err);
        assert_eq!(resp.status.as_u16(), 424);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["message"], expected_message);
    }
}
