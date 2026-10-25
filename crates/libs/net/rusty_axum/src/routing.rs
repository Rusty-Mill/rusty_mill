//! The router: paths to method routers to handlers.
//!
//! A path is static segments and `{name}` parameters (`/tasks/{id}`). When
//! several routes match, the one whose earliest segments are static wins,
//! so `/tasks/new` beats `/tasks/{id}`. A matched path with no handler for
//! the method answers 405 with `Allow`; `HEAD` falls back to `GET`. A
//! handler routed with [`any`] takes every method its path has no handler
//! of its own for, and [`Router::fallback`] takes every unmatched path.

use std::sync::Arc;

use rusty_http::{Method, StatusCode};

use crate::handler::{erase, BoxHandler, Handler};
use crate::middleware::{Middleware, Next};
use crate::request::Request;
use crate::response::{IntoResponse, Response};
use crate::BoxFuture;

/// Routes requests to handlers. `S` is the state its handlers extract with
/// [`State`](crate::State), supplied by [`Router::with_state`]; a router
/// with every state supplied is a `Router<()>`, which [`Router::call`]s.
/// Cloning is cheap: routes and layers are shared.
pub struct Router<S = ()> {
    routes: Arc<Vec<Route<S>>>,
    fallback: Option<BoxHandler<S>>,
    layers: Arc<Vec<Arc<dyn Middleware>>>,
}

impl<S> Clone for Router<S> {
    fn clone(&self) -> Self {
        Router {
            routes: self.routes.clone(),
            fallback: self.fallback.clone(),
            layers: self.layers.clone(),
        }
    }
}

struct Route<S> {
    path: String,
    pattern: Vec<Segment>,
    methods: MethodRouter<S>,
}

/// Captured path parameters, name and decoded value, in route order.
type Params = Vec<(String, String)>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Static(String),
    Param(String),
}

impl<S> Clone for Route<S> {
    fn clone(&self) -> Self {
        Route {
            path: self.path.clone(),
            pattern: self.pattern.clone(),
            methods: self.methods.clone(),
        }
    }
}

impl<S> Default for Router<S> {
    fn default() -> Self {
        Router {
            routes: Arc::new(Vec::new()),
            fallback: None,
            layers: Arc::new(Vec::new()),
        }
    }
}

impl<S: Send + Sync + 'static> Router<S> {
    /// An empty router: every request is 404.
    pub fn new() -> Self {
        Router::default()
    }

    /// Routes `path` to `methods`. A path given twice merges its methods.
    ///
    /// # Panics
    ///
    /// When `path` does not start with `/`, has an unsupported segment
    /// (`:name`, `{*rest}`, an unclosed brace), repeats a parameter name,
    /// overlaps a route of the same shape under other parameter names, or
    /// gives a method a second handler. These are mistakes in the program,
    /// found the first time it builds its router.
    pub fn route(mut self, path: &str, methods: MethodRouter<S>) -> Self {
        let pattern = parse(path);
        let routes = Arc::make_mut(&mut self.routes);
        if let Some(route) = routes.iter_mut().find(|r| same_shape(&r.pattern, &pattern)) {
            assert!(
                route.path == path,
                "route {path:?} overlaps {:?}",
                route.path
            );
            route.methods.merge(methods, path);
            return self;
        }
        routes.push(Route {
            path: path.to_string(),
            pattern,
            methods,
        });
        self
    }

    /// Answers every request whose path matches no route with `handler`,
    /// instead of 404. Layers still run around it. A second call replaces
    /// the first.
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.fallback = Some(erase(handler));
        self
    }

    /// Adds every route of `other`, and its fallback.
    ///
    /// # Panics
    ///
    /// As [`Router::route`], when the two routers overlap; when both have a
    /// fallback; and when `other` has layers, which would silently apply to
    /// every route after the merge. Merge first, then add layers.
    pub fn merge(mut self, other: Router<S>) -> Self {
        assert!(
            other.layers.is_empty(),
            "merge a router before adding its layers"
        );
        if let Some(fallback) = other.fallback {
            assert!(self.fallback.is_none(), "both routers have a fallback");
            self.fallback = Some(fallback);
        }
        other.routes.iter().fold(self, |router, route| {
            router.route(&route.path, route.methods.clone())
        })
    }

    /// Wraps every request, including those answered 404 or 405, in
    /// `middleware`. The last layer added runs first, as in axum.
    pub fn layer(mut self, middleware: impl Middleware) -> Self {
        Arc::make_mut(&mut self.layers).push(Arc::new(middleware));
        self
    }

    /// Supplies the state every handler extracts, giving a router that
    /// needs `S2` instead (usually `()`).
    pub fn with_state<S2>(self, state: S) -> Router<S2>
    where
        S: Clone,
    {
        let routes = self
            .routes
            .iter()
            .map(|route| Route {
                path: route.path.clone(),
                pattern: route.pattern.clone(),
                methods: route.methods.clone().with_state(state.clone()),
            })
            .collect();
        Router {
            routes: Arc::new(routes),
            fallback: self.fallback.map(|handler| bind(handler, state)),
            layers: self.layers,
        }
    }
}

impl Router<()> {
    /// Answers `req` in process: layers, routing, extraction and the
    /// handler, no socket. The server calls this; so do tests. A `HEAD`
    /// answer keeps the status and headers of the `GET` answer but no body:
    /// a whole body becomes its `Content-Length`, and a stream is dropped
    /// unpolled. That happens outside every layer, so layers see the body.
    pub fn call(&self, req: Request) -> BoxFuture<'static, Response> {
        let head = *req.method() == Method::Head;
        let response = Next::new(self.clone(), self.layers.len()).run(req);
        if !head {
            return response;
        }
        Box::pin(async move { without_body(response.await) })
    }

    /// The `i`th layer, counted from the first added.
    pub(crate) fn layer_at(&self, i: usize) -> Option<&Arc<dyn Middleware>> {
        self.layers.get(i)
    }

    /// Routing and the handler, after every layer.
    pub(crate) fn dispatch(&self, mut req: Request) -> BoxFuture<'static, Response> {
        let Some(segments) = decode_segments(req.path()) else {
            return ready((StatusCode::BAD_REQUEST, "path is not valid UTF-8"));
        };
        let Some((route, params)) = self.best_match(&segments) else {
            return match &self.fallback {
                Some(fallback) => fallback(req, &()),
                None => ready(StatusCode::NOT_FOUND),
            };
        };
        let Some(handler) = route.methods.find(req.method()) else {
            return ready(route.methods.not_allowed());
        };
        req.set_params(params);
        handler(req, &())
    }

    fn best_match(&self, segments: &[String]) -> Option<(&Route<()>, Params)> {
        self.routes
            .iter()
            .filter_map(|route| capture(&route.pattern, segments).map(|params| (route, params)))
            .max_by_key(|(route, _)| specificity(&route.pattern))
    }
}

/// A `HEAD` answer: the body goes, its length stays when it is known.
fn without_body(response: Response) -> Response {
    let length = response.body.as_bytes().map(<[u8]>::len);
    let mut response = Response {
        body: crate::Body::empty(),
        ..response
    };
    if let Some(length) = length {
        response = response.with_header("content-length", &length.to_string());
    }
    response
}

fn ready(response: impl IntoResponse) -> BoxFuture<'static, Response> {
    let response = response.into_response();
    Box::pin(async move { response })
}

fn parse(path: &str) -> Vec<Segment> {
    assert!(path.starts_with('/'), "route {path:?} must start with '/'");
    let pattern: Vec<Segment> = path[1..]
        .split('/')
        .map(
            |segment| match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(name) => {
                    assert!(
                        !name.is_empty() && !name.contains(['{', '}', '*']),
                        "route {path:?}: unsupported parameter {segment:?}"
                    );
                    Segment::Param(name.to_string())
                }
                None => {
                    assert!(
                        !segment.contains(['{', '}']) && !segment.starts_with(':'),
                        "route {path:?}: unsupported segment {segment:?} (parameters are {{name}})"
                    );
                    Segment::Static(segment.to_string())
                }
            },
        )
        .collect();
    let mut names: Vec<&str> = pattern
        .iter()
        .filter_map(|s| match s {
            Segment::Param(name) => Some(name.as_str()),
            Segment::Static(_) => None,
        })
        .collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert!(
        names.len() == count,
        "route {path:?} repeats a parameter name"
    );
    pattern
}

fn same_shape(a: &[Segment], b: &[Segment]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (Segment::Static(x), Segment::Static(y)) => x == y,
            (Segment::Param(_), Segment::Param(_)) => true,
            _ => false,
        })
}

/// The request path's segments, percent-decoded; `None` when one is not
/// UTF-8.
fn decode_segments(path: &str) -> Option<Vec<String>> {
    path.strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .map(rusty_percent::decode_utf8)
        .collect()
}

fn capture(pattern: &[Segment], segments: &[String]) -> Option<Params> {
    if pattern.len() != segments.len() {
        return None;
    }
    let mut params = Vec::new();
    for (expected, actual) in pattern.iter().zip(segments) {
        match expected {
            Segment::Static(s) if s == actual => {}
            Segment::Static(_) => return None,
            Segment::Param(name) => params.push((name.clone(), actual.clone())),
        }
    }
    Some(params)
}

/// Static segments outrank parameters, earliest segment first.
fn specificity(pattern: &[Segment]) -> Vec<bool> {
    pattern
        .iter()
        .map(|s| matches!(s, Segment::Static(_)))
        .collect()
}

/// Handlers for one path, by method. Built with [`get`], [`post`] and the
/// others, chained: `get(list).post(create)`.
pub struct MethodRouter<S = ()> {
    handlers: Vec<(Method, BoxHandler<S>)>,
    any: Option<BoxHandler<S>>,
}

impl<S> Clone for MethodRouter<S> {
    fn clone(&self) -> Self {
        MethodRouter {
            handlers: self.handlers.clone(),
            any: self.any.clone(),
        }
    }
}

impl<S> Default for MethodRouter<S> {
    fn default() -> Self {
        MethodRouter {
            handlers: Vec::new(),
            any: None,
        }
    }
}

/// Routes every method to `handler`, as axum's `any`. Methods given their
/// own handler on the same path keep it.
pub fn any<H, T, S>(handler: H) -> MethodRouter<S>
where
    H: Handler<T, S>,
    T: 'static,
    S: Send + Sync + 'static,
{
    MethodRouter {
        handlers: Vec::new(),
        any: Some(erase(handler)),
    }
}

/// `handler` with `state` supplied, needing `S2` instead.
fn bind<S, S2>(handler: BoxHandler<S>, state: S) -> BoxHandler<S2>
where
    S: Clone + Send + Sync + 'static,
{
    Arc::new(move |req, _: &S2| handler(req, &state))
}

impl<S: Send + Sync + 'static> MethodRouter<S> {
    fn on<H, T>(mut self, method: Method, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        assert!(
            self.handlers.iter().all(|(m, _)| *m != method),
            "{} has two handlers",
            method.as_str()
        );
        self.handlers.push((method, erase(handler)));
        self
    }

    fn merge(&mut self, other: MethodRouter<S>, path: &str) {
        if let Some(any) = other.any {
            assert!(self.any.is_none(), "route {path:?}: `any` has two handlers");
            self.any = Some(any);
        }
        for (method, handler) in other.handlers {
            assert!(
                self.handlers.iter().all(|(m, _)| *m != method),
                "route {path:?}: {} has two handlers",
                method.as_str()
            );
            self.handlers.push((method, handler));
        }
    }

    fn with_state<S2>(self, state: S) -> MethodRouter<S2>
    where
        S: Clone,
    {
        let handlers = self
            .handlers
            .into_iter()
            .map(|(method, handler)| (method, bind(handler, state.clone())))
            .collect();
        MethodRouter {
            handlers,
            any: self.any.map(|handler| bind(handler, state)),
        }
    }
}

impl MethodRouter<()> {
    fn find(&self, method: &Method) -> Option<&BoxHandler<()>> {
        let exact = self.handlers.iter().find(|(m, _)| m == method);
        let head_as_get = || {
            (*method == Method::Head)
                .then(|| self.handlers.iter().find(|(m, _)| *m == Method::Get))
                .flatten()
        };
        exact
            .or_else(head_as_get)
            .map(|(_, handler)| handler)
            .or(self.any.as_ref())
    }

    fn not_allowed(&self) -> Response {
        let mut allow: Vec<&str> = self.handlers.iter().map(|(m, _)| m.as_str()).collect();
        if allow.contains(&"GET") && !allow.contains(&"HEAD") {
            allow.push("HEAD");
        }
        Response::new(StatusCode::METHOD_NOT_ALLOWED, crate::Body::empty())
            .with_header("allow", &allow.join(", "))
    }
}

macro_rules! method_fns {
    ($($name:ident => $method:ident),*) => {
        $(
            #[doc = concat!("Routes `", stringify!($method), "` to `handler`.")]
            pub fn $name<H, T, S>(handler: H) -> MethodRouter<S>
            where
                H: Handler<T, S>,
                T: 'static,
                S: Send + Sync + 'static,
            {
                MethodRouter::default().on(Method::$method, handler)
            }
        )*
        impl<S: Send + Sync + 'static> MethodRouter<S> {
            $(
                #[doc = concat!("Also routes `", stringify!($method), "` to `handler`.")]
                pub fn $name<H, T>(self, handler: H) -> Self
                where
                    H: Handler<T, S>,
                    T: 'static,
                {
                    self.on(Method::$method, handler)
                }
            )*
        }
    };
}

method_fns!(get => Get, post => Post, put => Put, patch => Patch, delete => Delete);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn static_beats_parameter() {
        let a = parse("/tasks/{id}");
        let b = parse("/tasks/new");
        let segs = vec!["tasks".to_string(), "new".to_string()];
        assert!(capture(&a, &segs).is_some() && capture(&b, &segs).is_some());
        assert!(specificity(&b) > specificity(&a));
    }

    #[test]
    fn root_is_one_empty_segment() {
        assert_eq!(parse("/"), vec![Segment::Static(String::new())]);
        assert_eq!(decode_segments("/"), Some(vec![String::new()]));
    }

    #[test]
    #[should_panic(expected = "unsupported segment")]
    fn colon_syntax_is_refused() {
        parse("/tasks/:id");
    }

    #[test]
    #[should_panic(expected = "repeats a parameter")]
    fn repeated_parameter_is_refused() {
        parse("/{a}/{a}");
    }

    #[test]
    #[should_panic(expected = "overlaps")]
    fn same_shape_other_names_is_refused() {
        let _ = Router::<()>::new()
            .route("/t/{a}", get(|| async {}))
            .route("/t/{b}", post(|| async {}));
    }

    #[test]
    #[should_panic(expected = "two handlers")]
    fn second_handler_for_a_method_is_refused() {
        let _ = Router::<()>::new()
            .route("/t", get(|| async {}))
            .route("/t", get(|| async {}));
    }
}
