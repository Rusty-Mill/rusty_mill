//! The request a handler sees.

use rusty_http::{HeaderMap, Method};

/// A request with its body read whole. The server builds one per request;
/// a test builds one with [`Request::new`] and hands it to
/// [`Router::call`](crate::Router::call).
#[derive(Debug, Clone)]
pub struct Request {
    method: Method,
    target: String,
    headers: HeaderMap,
    body: Vec<u8>,
    params: Vec<(String, String)>,
}

impl Request {
    /// A request for `target`, the origin-form path with an optional
    /// `?query`, with no headers and an empty body.
    pub fn new(method: Method, target: impl Into<String>) -> Self {
        Request {
            method,
            target: target.into(),
            headers: HeaderMap::new(),
            body: Vec::new(),
            params: Vec::new(),
        }
    }

    /// Adds a header.
    ///
    /// # Errors
    ///
    /// When the name or value is not a valid header.
    pub fn with_header(mut self, name: &str, value: &str) -> rusty_http::Result<Self> {
        self.headers.append(name, value)?;
        Ok(self)
    }

    /// Replaces the headers.
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Sets the body.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// The method.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The request target as received: path and optional `?query`.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The path, without the query.
    pub fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(path, _)| path)
    }

    /// The raw query string, without the `?`.
    pub fn query(&self) -> Option<&str> {
        self.target.split_once('?').map(|(_, query)| query)
    }

    /// The headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The headers, for middleware that changes them.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// The body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Takes the body, leaving it empty.
    pub fn take_body(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.body)
    }

    /// The path parameters the matched route captured, in route order,
    /// percent-decoded. Empty before routing.
    pub fn params(&self) -> &[(String, String)] {
        &self.params
    }

    pub(crate) fn set_params(&mut self, params: Vec<(String, String)>) {
        self.params = params;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn splits_path_and_query() {
        let req = Request::new(Method::Get, "/stream?session=a&x=1");
        assert_eq!(req.path(), "/stream");
        assert_eq!(req.query(), Some("session=a&x=1"));
        let bare = Request::new(Method::Get, "/health");
        assert_eq!(bare.path(), "/health");
        assert_eq!(bare.query(), None);
    }

    #[test]
    fn take_body_leaves_it_empty() {
        let mut req = Request::new(Method::Post, "/").with_body(b"abc".to_vec());
        assert_eq!(req.take_body(), b"abc");
        assert!(req.body().is_empty());
    }

    #[test]
    fn rejects_an_invalid_header() {
        assert!(Request::new(Method::Get, "/")
            .with_header("x", "a\r\nb")
            .is_err());
    }
}
