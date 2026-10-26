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
//! - **One server.** [`serve`] runs HTTP/1.1 on `rusty_tokio`, plain or
//!   over `rusty_tls`, and with the `http2` feature HTTP/2 on a TLS
//!   connection that negotiates ALPN `h2`. [`serve_with`] lets an
//!   [`Accept`] choose per connection (a certificate by SNI, passthrough).
//!   There is no `tokio` dependency.
//! - **`rusty_http`'s types.** [`Method`], [`StatusCode`] and [`HeaderMap`]
//!   are re-exported from it; there is no `http` crate.
//!
//! Not built (no consumer needs it, `RUSTY-AXUM-SCOPE.md` §6.1): WebSocket,
//! multipart, cookies, typed headers, cleartext h2c.

mod body;
pub mod cors;
mod de;
pub mod extract;
mod handler;
pub mod middleware;
mod request;
pub mod response;
pub mod routing;
pub mod serve;
mod stop;

pub use body::{Body, BodyStream, BoxError};
pub use extract::{ConnectInfo, Form, FromRequest, Json, Path, Query, Rejection, State};
/// The trait [`Body::from_stream`] takes, so a consumer can implement it
/// without depending on `futures-core` itself.
pub use futures_core::Stream;
pub use handler::Handler;
pub use request::Request;
pub use response::{IntoResponse, Response};
pub use routing::Router;
pub use rusty_http::{HeaderMap, Method, StatusCode};
pub use serve::{serve, serve_with, Accept, Accepted, Admission, ServeOptions, Server};

use std::future::Future;
use std::pin::Pin;

/// A boxed, sendable future: what a type-erased handler returns.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
