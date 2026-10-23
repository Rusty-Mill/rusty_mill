//! An in-house web framework replacing `axum` across the workspace (#879).
//!
//! The names follow axum's (`Router`, `get`, `State`, `Query`, `Json`,
//! `IntoResponse`, `Sse`), so moving a consumer is mostly a change of
//! imports. The model is smaller:
//!
//! - **Runtime-free core.** Routing, extraction and responses never touch a
//!   socket or a runtime; [`Router::call`] answers a [`Request`] in process,
//!   which is also how handlers are tested.
//! - **Buffered request bodies.** The server reads a body whole (bounded)
//!   before routing, so extraction is synchronous: a [`FromRequest`] reads
//!   or takes what it needs from `&mut Request`. Response bodies may stream.
//! - **`rusty_http`'s types.** [`Method`], [`StatusCode`] and [`HeaderMap`]
//!   are re-exported from it; there is no `http` crate.
//!
//! Not built (no consumer needs it, `RUSTY-AXUM-SCOPE.md` §6.1): WebSocket,
//! multipart, cookies, typed headers, HTTP/2 for an app.

mod body;
mod de;
pub mod extract;
mod handler;
mod request;
pub mod response;
pub mod routing;

pub use body::{Body, BoxError};
pub use extract::{FromRequest, Json, Path, Query, Rejection, State};
pub use handler::Handler;
pub use request::Request;
pub use response::{IntoResponse, Response};
pub use routing::Router;
pub use rusty_http::{HeaderMap, Method, StatusCode};

use std::future::Future;
use std::pin::Pin;

/// A boxed, sendable future: what a type-erased handler returns.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
