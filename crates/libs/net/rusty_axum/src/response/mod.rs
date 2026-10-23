//! Responses, and what a handler may return ([`IntoResponse`]).

pub mod sse;

use std::convert::Infallible;

use rusty_http::{HeaderMap, StatusCode};

use crate::body::Body;

/// A response: status, headers and body. The server sets the framing and
/// connection headers (`Content-Length`, `Transfer-Encoding`,
/// `Connection`) itself and replaces any set here.
#[derive(Debug)]
pub struct Response {
    /// The status.
    pub status: StatusCode,
    /// The headers.
    pub headers: HeaderMap,
    /// The body.
    pub body: Body,
}

impl Response {
    /// A response with no headers.
    pub fn new(status: StatusCode, body: impl Into<Body>) -> Self {
        Response {
            status,
            headers: HeaderMap::new(),
            body: body.into(),
        }
    }

    /// Sets a header, or answers 500 when it is invalid: a handler that
    /// builds a header from data must not send a malformed one.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        match self.headers.insert(name, value) {
            Ok(()) => self,
            Err(e) => Response::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("invalid response header {name:?}: {e}"),
            ),
        }
    }
}

/// Anything a handler may return.
pub trait IntoResponse {
    /// Converts `self` into a response.
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for StatusCode {
    fn into_response(self) -> Response {
        Response::new(self, Body::empty())
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response {
        StatusCode::OK.into_response()
    }
}

impl IntoResponse for Infallible {
    fn into_response(self) -> Response {
        match self {}
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, self).with_header("content-type", TEXT)
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, self).with_header("content-type", TEXT)
    }
}

impl IntoResponse for Vec<u8> {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, self).with_header("content-type", "application/octet-stream")
    }
}

impl IntoResponse for Body {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, self)
    }
}

impl<T: IntoResponse, E: IntoResponse> IntoResponse for Result<T, E> {
    fn into_response(self) -> Response {
        match self {
            Ok(ok) => ok.into_response(),
            Err(err) => err.into_response(),
        }
    }
}

/// Overrides the status of the inner response.
impl<R: IntoResponse> IntoResponse for (StatusCode, R) {
    fn into_response(self) -> Response {
        let mut response = self.1.into_response();
        response.status = self.0;
        response
    }
}

/// An HTML body, sent as `text/html; charset=utf-8`.
#[derive(Debug, Clone)]
pub struct Html<T>(pub T);

impl<T: Into<String>> IntoResponse for Html<T> {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, self.0.into())
            .with_header("content-type", "text/html; charset=utf-8")
    }
}

const TEXT: &str = "text/plain; charset=utf-8";

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn text_is_utf8_plain() {
        let r = "hi".into_response();
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.headers.get("content-type"), Some(TEXT));
        assert_eq!(r.body.as_bytes(), Some(&b"hi"[..]));
    }

    #[test]
    fn tuple_overrides_status_and_keeps_headers() {
        let r = (StatusCode::CREATED, Html("<p>")).into_response();
        assert_eq!(r.status, StatusCode::CREATED);
        assert_eq!(
            r.headers.get("content-type"),
            Some("text/html; charset=utf-8")
        );
    }

    #[test]
    fn result_answers_either_side() {
        let ok: Result<&'static str, StatusCode> = Ok("x");
        let err: Result<&'static str, StatusCode> = Err(StatusCode::UNAUTHORIZED);
        assert_eq!(ok.into_response().status, StatusCode::OK);
        assert_eq!(err.into_response().status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn invalid_header_becomes_500() {
        let r = Response::new(StatusCode::OK, "x").with_header("etag", "a\nb");
        assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
