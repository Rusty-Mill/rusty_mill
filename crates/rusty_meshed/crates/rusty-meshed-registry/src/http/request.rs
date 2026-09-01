//! The request type handlers receive: method, path, decoded query
//! pairs, path params (filled in by [`super::router::Router`] once a
//! route matches), headers, and the raw body.

use super::query::split_target;
use rusty_http::head::RequestHead;
use rusty_http::{HeaderMap, Method};

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Request {
    /// Builds a `Request` from a parsed head and its already-read
    /// body. `params` starts empty -- [`super::router::Router::dispatch`]
    /// fills it in once it knows which route pattern matched.
    pub fn from_head(head: &RequestHead, body: Vec<u8>) -> Self {
        let (path, query) = split_target(&head.target);
        Request {
            method: head.method.clone(),
            path,
            query,
            params: Vec::new(),
            headers: head.headers.clone(),
            body,
        }
    }

    /// The first path parameter matching `name` (e.g. `{id}` in a
    /// registered pattern), if the matched route declared one by that
    /// name.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The first query parameter matching `name`, if present.
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Parses the body as JSON. `Err` for non-UTF-8 or malformed JSON
    /// -- a handler maps that to a 4xx response itself, matching how a
    /// framework-level "body doesn't parse" case is always a route
    /// handler's own concern here, not an implicit middleware layer.
    pub fn json(&self) -> Result<rusty_request::Json, String> {
        let text = std::str::from_utf8(&self.body).map_err(|err| err.to_string())?;
        rusty_request::Json::parse(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(target: &str) -> RequestHead {
        RequestHead {
            method: Method::Get,
            target: target.to_string(),
            version: rusty_http::Version::Http11,
            headers: HeaderMap::new(),
        }
    }

    #[test]
    fn from_head_splits_path_and_query() {
        let req = Request::from_head(&head("/data-products?name=orders"), Vec::new());
        assert_eq!(req.path, "/data-products");
        assert_eq!(req.query_param("name"), Some("orders"));
    }

    #[test]
    fn param_looks_up_by_name() {
        let mut req = Request::from_head(&head("/data-products/2"), Vec::new());
        req.params.push(("id".to_string(), "2".to_string()));
        assert_eq!(req.param("id"), Some("2"));
        assert_eq!(req.param("missing"), None);
    }

    #[test]
    fn json_parses_a_valid_body() {
        let req = Request::from_head(&head("/"), br#"{"name":"orders"}"#.to_vec());
        let value = req.json().unwrap();
        assert_eq!(value.get("name").unwrap().as_str(), Some("orders"));
    }

    #[test]
    fn json_rejects_a_malformed_body() {
        let req = Request::from_head(&head("/"), b"not json".to_vec());
        assert!(req.json().is_err());
    }
}
