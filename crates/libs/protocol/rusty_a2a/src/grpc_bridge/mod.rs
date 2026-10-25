//! Runs the gRPC transport for callers on `tokio`.
//!
//! The first-party gRPC stack (`rusty_tonic` over `rusty_hyper` and `rusty_h2`) is async on
//! `rusty_tokio`, its own runtime. This crate's callers, its engine, `axum` and `reqwest` run on
//! `tokio`. The two meet here and nowhere else, by message passing only: a task on the transport's
//! runtime is awaited from `tokio` through its `JoinHandle`, and a stream crosses through a
//! `tokio::sync::mpsc` channel. Neither side enters the other's runtime context.
//!
//! Dropping the future a caller was awaiting, or the stream it was reading, aborts the transport
//! task, which drops the call and so resets its HTTP/2 stream: cancelling a call from `tokio`
//! cancels it on the wire.
//!
//! The client half is [`client`], the server half [`server`]. When this crate leaves `tokio`
//! altogether, this module goes with it.

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
pub(crate) use client::{server_streaming, unary};

#[cfg(feature = "server")]
pub(crate) mod server;
