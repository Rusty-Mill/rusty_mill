//! An MCP client core on `rusty_mcp_proto`.
//!
//! Layers, each usable alone:
//!
//! - [`sse`]: a server-sent-events parser (pure).
//! - [`ClientSession`]: request ids, the classic and stateless handshakes and
//!   the stateless per-request `_meta`, without any I/O.
//! - [`Transport`] and [`StdioTransport`]: JSON-RPC messages over a child
//!   process or any pair of byte streams.
//! - [`Client`]: a blocking client on top, with typed calls, multi-round-trip
//!   input and tasks driven to a result.
//!
//! - `HttpTransport` (feature `http`): Streamable HTTP on `rusty_request`, with
//!   JSON and event-stream replies, sessions, the standalone push stream with
//!   resumption, and hang-up cancellation.
//!
//! - [`AsyncClient`]: the blocking client on a worker thread behind futures,
//!   usable from any executor.

#![forbid(unsafe_code)]

pub mod asynchronous;
pub mod client;
pub mod error;
#[cfg(feature = "http")]
pub mod http;
pub mod session;
pub mod sse;
pub mod stdio;
pub mod transport;

pub use asynchronous::{AsyncClient, Reply};
pub use client::{Client, Handler, NoHandler};
pub use error::ClientError;
#[cfg(feature = "http")]
pub use http::{HttpConfig, HttpTransport};
/// The JSON value type the wire types are built on.
pub use rusty_json as json;
/// The wire types, re-exported so a consumer needs no dependency of its own.
pub use rusty_mcp_proto as proto;
pub use session::{ClientConfig, ClientSession, Incoming};
pub use stdio::StdioTransport;
pub use transport::{Recv, Transport};
