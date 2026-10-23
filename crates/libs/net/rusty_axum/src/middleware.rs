//! Middleware: code around every request of a router
//! ([`Router::layer`](crate::Router::layer)).

use std::future::Future;

use crate::request::Request;
use crate::response::{IntoResponse, Response};
use crate::routing::Router;
use crate::BoxFuture;

/// Wraps a router's requests. Most are written with [`from_fn`]; implement
/// it directly for one that carries configuration, as [`Cors`](crate::cors::Cors) does.
pub trait Middleware: Send + Sync + 'static {
    /// Handles `req`, usually by passing it on with [`Next::run`] and
    /// changing the response, or by answering without passing it on.
    fn call(&self, req: Request, next: Next) -> BoxFuture<'static, Response>;
}

/// The rest of the chain after a middleware: the inner layers, then routing
/// and the handler.
pub struct Next {
    router: Router,
    depth: usize,
}

impl Next {
    pub(crate) fn new(router: Router, depth: usize) -> Self {
        Next { router, depth }
    }

    /// Runs the rest of the chain.
    pub fn run(self, req: Request) -> BoxFuture<'static, Response> {
        let Some(inner) = self.depth.checked_sub(1) else {
            return self.router.dispatch(req);
        };
        match self.router.layer_at(inner).cloned() {
            Some(layer) => layer.call(req, Next::new(self.router, inner)),
            None => self.router.dispatch(req),
        }
    }
}

/// Middleware from an async function of the request and [`Next`], as
/// axum's `middleware::from_fn`.
pub fn from_fn<F, Fut, R>(f: F) -> FromFn<F>
where
    F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: IntoResponse,
{
    FromFn(f)
}

/// See [`from_fn`].
#[derive(Debug, Clone)]
pub struct FromFn<F>(F);

impl<F, Fut, R> Middleware for FromFn<F>
where
    F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: IntoResponse,
{
    fn call(&self, req: Request, next: Next) -> BoxFuture<'static, Response> {
        let fut = (self.0)(req, next);
        Box::pin(async move { fut.await.into_response() })
    }
}
