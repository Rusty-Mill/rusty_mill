//! The response type handlers return: a status, headers, and a body.
//! `Content-Length` is set by [`super::server::serve`] right before
//! writing, from the final body length -- builders here don't need to
//! track it themselves.

use rusty_http::{HeaderMap, StatusCode};

#[derive(Debug, Clone)]
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Response {
    /// An empty response with the given status and no body.
    pub fn new(status: StatusCode) -> Self {
        Response {
            status,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    /// A `Content-Type: application/json` response serializing `value`.
    pub fn json(status: StatusCode, value: &rusty_request::Json) -> Self {
        let mut response = Response::new(status);
        response
            .headers
            .insert("Content-Type", "application/json")
            .expect("static header name/value is always valid");
        response.body = value.to_json_string().into_bytes();
        response
    }

    /// A `Content-Type: text/plain; charset=utf-8` response.
    pub fn text(status: StatusCode, body: impl Into<String>) -> Self {
        let mut response = Response::new(status);
        response
            .headers
            .insert("Content-Type", "text/plain; charset=utf-8")
            .expect("static header name/value is always valid");
        response.body = body.into().into_bytes();
        response
    }

    /// A `Content-Type: text/html; charset=utf-8` response.
    pub fn html(status: StatusCode, body: impl Into<String>) -> Self {
        let mut response = Response::new(status);
        response
            .headers
            .insert("Content-Type", "text/html; charset=utf-8")
            .expect("static header name/value is always valid");
        response.body = body.into().into_bytes();
        response
    }

    pub fn not_found() -> Self {
        Response::text(StatusCode::NOT_FOUND, "Not Found")
    }

    pub fn method_not_allowed() -> Self {
        Response::text(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_sets_content_type_and_serializes_the_value() {
        let mut value = rusty_request::Json::object();
        value.insert("ok", true);
        let response = Response::json(StatusCode::OK, &value);
        assert_eq!(
            response.headers.get("Content-Type"),
            Some("application/json")
        );
        assert_eq!(response.body, br#"{"ok":true}"#);
    }

    #[test]
    fn text_and_html_set_the_expected_content_types() {
        let response = Response::text(StatusCode::OK, "hi");
        assert_eq!(
            response.headers.get("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(response.body, b"hi");

        let response = Response::html(StatusCode::OK, "<p>hi</p>");
        assert_eq!(
            response.headers.get("Content-Type"),
            Some("text/html; charset=utf-8")
        );
    }

    #[test]
    fn not_found_and_method_not_allowed_use_the_right_status() {
        assert_eq!(Response::not_found().status, StatusCode::NOT_FOUND);
        assert_eq!(
            Response::method_not_allowed().status,
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}
