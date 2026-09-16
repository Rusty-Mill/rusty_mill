//! CORS handling -- the Rust port of the source's
//! `CORSMiddleware(allow_origins=["http://localhost:5173"],
//! allow_methods=["*"], allow_headers=["*"])` (REG-005), which exists
//! for exactly one reason: letting the `data-mesh-monitor` Vite dev
//! server (a separate origin) call this API from a browser.

use super::request::Request;
use super::response::Response;
use super::router::Router;
use rusty_http::{Method, StatusCode};

/// The one origin the source's `CORSMiddleware` allows -- the Vite dev
/// server for `data-mesh-monitor` (out of scope for this migration,
/// see `capability-manifest.md`'s DASH rows, but the API still needs
/// to let it connect).
pub const ALLOWED_ORIGIN: &str = "http://localhost:5173";

/// Wraps route dispatch with CORS handling: an `OPTIONS` request (a
/// browser's preflight) short-circuits to an empty 200 without
/// reaching the router at all, and every response gets
/// `Access-Control-Allow-*` headers attached when the request's
/// `Origin` is the allowed one. A request from any other origin (or
/// with no `Origin` header, e.g. a same-origin or non-browser client)
/// is dispatched normally with no CORS headers added -- matching a
/// browser's own enforcement point being the header's *absence*, not
/// this server rejecting the request.
pub async fn handle(router: &Router, req: Request) -> Response {
    let origin_allowed = req
        .headers
        .get("Origin")
        .map(|origin| origin == ALLOWED_ORIGIN)
        .unwrap_or(false);

    if req.method == Method::Options {
        let mut response = Response::new(StatusCode::OK);
        if origin_allowed {
            add_headers(&mut response);
        }
        return response;
    }

    let mut response = router.dispatch(req).await;
    if origin_allowed {
        add_headers(&mut response);
    }
    response
}

fn add_headers(response: &mut Response) {
    response
        .headers
        .insert("Access-Control-Allow-Origin", ALLOWED_ORIGIN)
        .expect("static header name/value is always valid");
    response
        .headers
        .insert("Access-Control-Allow-Methods", "*")
        .expect("static header name/value is always valid");
    response
        .headers
        .insert("Access-Control-Allow-Headers", "*")
        .expect("static header name/value is always valid");
}

#[cfg(test)]
mod tests {
    use super::super::response::Response as HttpResponse;
    use super::*;
    use rusty_http::HeaderMap;

    fn router() -> Router {
        Router::new().get("/health", |_req| async {
            HttpResponse::text(StatusCode::OK, "ok")
        })
    }

    fn req_with_origin(method: Method, origin: Option<&str>) -> Request {
        let mut headers = HeaderMap::new();
        if let Some(origin) = origin {
            headers.insert("Origin", origin).unwrap();
        }
        Request {
            method,
            path: "/health".to_string(),
            query: Vec::new(),
            params: Vec::new(),
            headers,
            body: Vec::new(),
        }
    }

    #[rusty_tokio::test]
    async fn adds_cors_headers_when_origin_is_allowed() {
        let response = handle(
            &router(),
            req_with_origin(Method::Get, Some(ALLOWED_ORIGIN)),
        )
        .await;
        assert_eq!(
            response.headers.get("Access-Control-Allow-Origin"),
            Some(ALLOWED_ORIGIN)
        );
        assert_eq!(response.body, b"ok");
    }

    #[rusty_tokio::test]
    async fn omits_cors_headers_for_a_disallowed_origin() {
        let response = handle(
            &router(),
            req_with_origin(Method::Get, Some("http://evil.example")),
        )
        .await;
        assert_eq!(response.headers.get("Access-Control-Allow-Origin"), None);
    }

    #[rusty_tokio::test]
    async fn omits_cors_headers_with_no_origin_header() {
        let response = handle(&router(), req_with_origin(Method::Get, None)).await;
        assert_eq!(response.headers.get("Access-Control-Allow-Origin"), None);
    }

    #[rusty_tokio::test]
    async fn options_preflight_short_circuits_without_reaching_the_router() {
        let response = handle(
            &router(),
            req_with_origin(Method::Options, Some(ALLOWED_ORIGIN)),
        )
        .await;
        assert_eq!(response.status, StatusCode::OK);
        assert!(response.body.is_empty());
        assert_eq!(
            response.headers.get("Access-Control-Allow-Methods"),
            Some("*")
        );
    }
}
