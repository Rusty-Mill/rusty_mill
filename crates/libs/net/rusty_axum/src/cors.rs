//! CORS, as `tower-http`'s `CorsLayer` answers it: preflight requests are
//! answered here and never reach a handler; other requests get
//! `Access-Control-Allow-Origin` when their origin is allowed.

use std::time::Duration;

use rusty_http::{Method, StatusCode};

use crate::body::Body;
use crate::middleware::{Middleware, Next};
use crate::request::Request;
use crate::response::Response;
use crate::BoxFuture;

/// Anything, or exactly these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allow<T> {
    /// Any value (`*`).
    Any,
    /// Only these values; empty allows none.
    List(Vec<T>),
}

impl<T> Default for Allow<T> {
    fn default() -> Self {
        Allow::List(Vec::new())
    }
}

/// CORS middleware for [`Router::layer`](crate::Router::layer). Built like
/// `tower-http`'s: `Cors::new().allow_origin(..).allow_methods(..)`.
#[derive(Debug, Clone, Default)]
pub struct Cors {
    origins: Allow<String>,
    methods: Allow<Method>,
    headers: Allow<String>,
    max_age: Option<Duration>,
}

impl Cors {
    /// Allows nothing until configured.
    pub fn new() -> Self {
        Cors::default()
    }

    /// Any origin, method and header: `tower-http`'s `permissive`.
    pub fn permissive() -> Self {
        Cors {
            origins: Allow::Any,
            methods: Allow::Any,
            headers: Allow::Any,
            max_age: None,
        }
    }

    /// Which origins may read responses. Compared exactly, as sent in the
    /// `Origin` header (`https://app.example`).
    pub fn allow_origin(mut self, origins: Allow<String>) -> Self {
        self.origins = origins;
        self
    }

    /// Which methods a preflight allows.
    pub fn allow_methods(mut self, methods: Allow<Method>) -> Self {
        self.methods = methods;
        self
    }

    /// Which request headers a preflight allows; matched without case.
    pub fn allow_headers(mut self, headers: Allow<String>) -> Self {
        self.headers = headers;
        self
    }

    /// How long a browser may cache a preflight answer.
    pub fn max_age(mut self, max_age: Duration) -> Self {
        self.max_age = Some(max_age);
        self
    }

    /// The `Access-Control-Allow-Origin` value for `origin`, if allowed.
    fn allowed_origin(&self, origin: Option<&str>) -> Option<String> {
        match (&self.origins, origin) {
            (Allow::Any, _) => Some("*".to_string()),
            (Allow::List(list), Some(origin)) => {
                list.iter().any(|o| o == origin).then(|| origin.to_string())
            }
            (Allow::List(_), None) => None,
        }
    }

    fn preflight(&self, req: &Request) -> Response {
        let mut response = Response::new(StatusCode::OK, Body::empty());
        let methods = match &self.methods {
            Allow::Any => "*".to_string(),
            Allow::List(list) => join(list.iter().map(Method::as_str)),
        };
        let headers = match &self.headers {
            Allow::Any => "*".to_string(),
            Allow::List(list) => join(list.iter().map(String::as_str)),
        };
        if !methods.is_empty() {
            response = response.with_header("access-control-allow-methods", &methods);
        }
        if !headers.is_empty() {
            response = response.with_header("access-control-allow-headers", &headers);
        }
        if let Some(age) = self.max_age {
            response = response.with_header("access-control-max-age", &age.as_secs().to_string());
        }
        decorate(response, self.allowed_origin(req.headers().get("origin")))
    }
}

fn decorate(mut response: Response, allowed_origin: Option<String>) -> Response {
    if let Some(allowed) = allowed_origin {
        response = response.with_header("access-control-allow-origin", &allowed);
    }
    let vary = merge_vary(response.headers.get_all("vary"));
    response.with_header("vary", &vary)
}

/// What a CORS answer depends on, so caches keep one per origin.
const VARY: [&str; 3] = [
    "origin",
    "access-control-request-method",
    "access-control-request-headers",
];

/// The `Vary` value with the CORS fields added to what the response
/// already varies on. `*` already covers everything and stays alone.
fn merge_vary<'a>(existing: impl Iterator<Item = &'a str>) -> String {
    let mut tokens: Vec<&str> = existing
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.contains(&"*") {
        return "*".to_string();
    }
    for field in VARY {
        if !tokens.iter().any(|t| t.eq_ignore_ascii_case(field)) {
            tokens.push(field);
        }
    }
    tokens.join(", ")
}

fn join<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items.collect::<Vec<_>>().join(",")
}

impl Middleware for Cors {
    fn call(&self, req: Request, next: Next) -> BoxFuture<'static, Response> {
        let is_preflight = *req.method() == Method::Options
            && req.headers().contains("access-control-request-method");
        if is_preflight {
            let response = self.preflight(&req);
            return Box::pin(async move { response });
        }
        let allowed = self.allowed_origin(req.headers().get("origin"));
        let inner = next.run(req);
        Box::pin(async move { decorate(inner.await, allowed) })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn exact() -> Cors {
        Cors::new()
            .allow_origin(Allow::List(vec!["https://app.example".into()]))
            .allow_methods(Allow::List(vec![Method::Get, Method::Post]))
            .allow_headers(Allow::List(vec![
                "authorization".into(),
                "x-session-id".into(),
            ]))
    }

    #[test]
    fn only_a_listed_origin_is_echoed() {
        let cors = exact();
        assert_eq!(
            cors.allowed_origin(Some("https://app.example")).as_deref(),
            Some("https://app.example")
        );
        assert_eq!(cors.allowed_origin(Some("https://evil.example")), None);
        assert_eq!(cors.allowed_origin(None), None);
        assert_eq!(
            Cors::permissive().allowed_origin(None).as_deref(),
            Some("*")
        );
        assert_eq!(
            Cors::new().allowed_origin(Some("https://app.example")),
            None
        );
    }

    fn with_vary(vary: &str, origin: Option<&str>) -> Option<String> {
        let response = Response::new(StatusCode::OK, "x").with_header("vary", vary);
        let allowed = exact().allowed_origin(origin);
        decorate(response, allowed)
            .headers
            .get("vary")
            .map(str::to_string)
    }

    #[test]
    fn existing_vary_is_merged_not_skipped() {
        let all = "accept-encoding, origin, access-control-request-method, access-control-request-headers";
        for origin in [
            Some("https://app.example"),
            Some("https://evil.example"),
            None,
        ] {
            assert_eq!(with_vary("accept-encoding", origin).as_deref(), Some(all));
        }
        assert_eq!(
            with_vary("Origin, accept-encoding", None).as_deref(),
            Some("Origin, accept-encoding, access-control-request-method, access-control-request-headers")
        );
        assert_eq!(
            with_vary("*", Some("https://app.example")).as_deref(),
            Some("*")
        );
        assert_eq!(with_vary("accept, *", None).as_deref(), Some("*"));
    }

    #[test]
    fn preflight_lists_methods_and_headers() {
        let req = Request::new(Method::Options, "/chat")
            .with_header("origin", "https://app.example")
            .unwrap();
        let r = exact().max_age(Duration::from_secs(600)).preflight(&req);
        assert_eq!(
            r.headers.get("access-control-allow-methods"),
            Some("GET,POST")
        );
        assert_eq!(
            r.headers.get("access-control-allow-headers"),
            Some("authorization,x-session-id")
        );
        assert_eq!(r.headers.get("access-control-max-age"), Some("600"));
        assert_eq!(
            r.headers.get("access-control-allow-origin"),
            Some("https://app.example")
        );
        assert_eq!(r.headers.get("vary"), Some(VARY.join(", ").as_str()));
    }
}
