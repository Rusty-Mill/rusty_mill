//! Path-pattern route registration and dispatch.
//!
//! A pattern is a `/`-separated list of literal segments and
//! `{name}` params, e.g. `/data-products/{id}/output-ports/{port_id}`.
//! Matching is exact segment-count, in registration order -- the first
//! route whose method and pattern both match wins. There is no
//! wildcard/catch-all segment; every registered pattern's shape is
//! known up front, which is all a hand-rolled REST API needs.

use super::request::Request;
use super::response::Response;
use rusty_http::Method;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type Handler = Arc<dyn Fn(Request) -> BoxFuture<'static, Response> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Param(String),
}

fn compile(pattern: &str) -> Vec<Segment> {
    pattern
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            match segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            {
                Some(name) => Segment::Param(name.to_string()),
                None => Segment::Literal(segment.to_string()),
            }
        })
        .collect()
}

fn path_segments(path: &str) -> Vec<&str> {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// Matches `path` against `pattern`, returning the extracted path
/// params on success.
fn matches(pattern: &[Segment], path: &str) -> Option<Vec<(String, String)>> {
    let actual = path_segments(path);
    if pattern.len() != actual.len() {
        return None;
    }
    let mut params = Vec::new();
    for (segment, value) in pattern.iter().zip(actual.iter()) {
        match segment {
            Segment::Literal(literal) => {
                if literal != value {
                    return None;
                }
            }
            Segment::Param(name) => params.push((name.clone(), value.to_string())),
        }
    }
    Some(params)
}

struct Route {
    method: Method,
    pattern: String,
    segments: Vec<Segment>,
    handler: Handler,
}

/// A route table, matched in registration order. Cheap to build (a
/// `Vec`, no radix tree) -- this app's route count is small enough
/// that big-O doesn't matter, and registration order is what makes
/// "first match wins" predictable to reason about.
#[derive(Default)]
pub struct Router {
    routes: Vec<Route>,
}

impl Router {
    pub fn new() -> Self {
        Router { routes: Vec::new() }
    }

    /// Registers `handler` for `method` requests matching `pattern`.
    pub fn route<F, Fut>(mut self, method: Method, pattern: &str, handler: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.routes.push(Route {
            method,
            pattern: pattern.to_string(),
            segments: compile(pattern),
            handler: Arc::new(move |req| Box::pin(handler(req))),
        });
        self
    }

    pub fn get<F, Fut>(self, pattern: &str, handler: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.route(Method::Get, pattern, handler)
    }

    pub fn post<F, Fut>(self, pattern: &str, handler: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.route(Method::Post, pattern, handler)
    }

    pub fn patch<F, Fut>(self, pattern: &str, handler: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.route(Method::Patch, pattern, handler)
    }

    pub fn delete<F, Fut>(self, pattern: &str, handler: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.route(Method::Delete, pattern, handler)
    }

    /// Appends every route from `other` onto `self` -- the Rust
    /// equivalent of `app.include_router(...)` (REG-006): a future
    /// per-resource router (data-products, ports, ...) is built
    /// standalone and merged in here.
    pub fn merge(mut self, other: Router) -> Self {
        self.routes.extend(other.routes);
        self
    }

    /// `(method, pattern)` for every registered route, in registration
    /// order -- used to generate `/openapi.json`'s `paths` (REG-001,
    /// REG-137).
    pub fn routes(&self) -> Vec<(Method, String)> {
        self.routes
            .iter()
            .map(|route| (route.method.clone(), route.pattern.clone()))
            .collect()
    }

    /// Dispatches `req` to the first route whose pattern and method
    /// both match. A pattern match with no method match yields 405,
    /// matching REST convention (the resource exists, this verb on it
    /// doesn't); no pattern match at all yields 404.
    pub async fn dispatch(&self, mut req: Request) -> Response {
        let path = req.path.clone();
        let mut path_matched = false;
        for route in &self.routes {
            let Some(params) = matches(&route.segments, &path) else {
                continue;
            };
            path_matched = true;
            if route.method == req.method {
                req.params = params;
                return (route.handler)(req).await;
            }
        }
        if path_matched {
            Response::method_not_allowed()
        } else {
            Response::not_found()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::StatusCode;

    fn get_req(path: &str) -> Request {
        Request {
            method: Method::Get,
            path: path.to_string(),
            query: Vec::new(),
            params: Vec::new(),
            headers: rusty_http::HeaderMap::new(),
            body: Vec::new(),
        }
    }

    fn router() -> Router {
        Router::new()
            .get("/health", |_req| async {
                Response::text(StatusCode::OK, "ok")
            })
            .get("/data-products/{id}", |req| async move {
                let id = req.param("id").unwrap_or("").to_string();
                Response::text(StatusCode::OK, format!("product {id}"))
            })
    }

    #[rusty_tokio::test]
    async fn dispatches_a_literal_route() {
        let response = router().dispatch(get_req("/health")).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, b"ok");
    }

    #[rusty_tokio::test]
    async fn dispatches_a_route_with_a_path_param() {
        let response = router().dispatch(get_req("/data-products/42")).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, b"product 42");
    }

    #[rusty_tokio::test]
    async fn unmatched_path_is_404() {
        let response = router().dispatch(get_req("/nope")).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[rusty_tokio::test]
    async fn matched_path_wrong_method_is_405() {
        let mut req = get_req("/health");
        req.method = Method::Post;
        let response = router().dispatch(req).await;
        assert_eq!(response.status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn routes_reflects_every_registration_in_order() {
        let routes = router().routes();
        assert_eq!(
            routes,
            vec![
                (Method::Get, "/health".to_string()),
                (Method::Get, "/data-products/{id}".to_string()),
            ]
        );
    }

    #[test]
    fn merge_appends_routes_from_another_router() {
        let a = Router::new().get("/a", |_req| async { Response::text(StatusCode::OK, "a") });
        let b = Router::new().get("/b", |_req| async { Response::text(StatusCode::OK, "b") });
        let merged = a.merge(b);
        assert_eq!(
            merged.routes(),
            vec![
                (Method::Get, "/a".to_string()),
                (Method::Get, "/b".to_string()),
            ]
        );
    }
}
