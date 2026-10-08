//! An MCP server core on `rusty_mcp_proto`: describe a server and its tools
//! once, serve it to any number of clients.
//!
//! ```
//! use rusty_json::Value;
//! use rusty_mcp_proto::{CallToolResult, ContentBlock, Tool};
//! use rusty_mcp_server::Server;
//!
//! let mut schema = Value::object();
//! schema.insert("type", "object");
//! let server = Server::builder("demo", "0.1.0")
//!     .tool(Tool::new("hello", schema), |_ctx, _call| {
//!         Ok(CallToolResult {
//!             content: vec![ContentBlock::text("hello")],
//!             ..CallToolResult::default()
//!         })
//!     })
//!     .build()
//!     .unwrap();
//! # let _ = server;
//! // rusty_mcp_server::serve_stdio(std::sync::Arc::new(server))
//! ```
//!
//! Layers, each usable alone:
//!
//! - [`Server`] and [`ServerBuilder`]: the immutable description, with a
//!   tool registry. Schemas are advertised as given, not enforced.
//! - [`Connection`]: one client's conversation, without I/O. Serves the
//!   classic (`initialize`) and stateless (`server/discover`, per-request
//!   `_meta`) handshakes from the same handlers, with cancellation, progress
//!   and cursor pagination. Transports are thin loops over
//!   [`Connection::start`].
//! - [`stdio`]: newline-delimited JSON on stdin/stdout, requests running
//!   concurrently; end of input drains them for a bounded time, then cancels.
//!
//! Not here yet: prompts, resources, completion, subscriptions, tasks and
//! multi-round-trip input on the server side, and the Streamable HTTP
//! transport.

#![forbid(unsafe_code)]

pub mod connection;
#[cfg(feature = "http")]
pub mod http;
mod page;
pub mod server;
pub mod stdio;

/// The JSON value type the protocol types are built on, re-exported so a
/// consumer of this crate needs no dependency of its own for it.
pub use rusty_json as json;
/// The wire types (`Tool`, `CallToolResult`, `ContentBlock`, ...), re-exported
/// for the same reason.
pub use rusty_mcp_proto as proto;

pub use connection::{CallContext, CancelToken, Connection, Job, Notifier, Started};
#[cfg(feature = "http")]
pub use http::{bind_http, HttpConfig, HttpHandler};
pub use server::{BuildError, Server, ServerBuilder, ToolHandler};
pub use stdio::{
    serve_lines, serve_stdio, StdioConfig, DEFAULT_DRAIN_TIMEOUT, DEFAULT_MAX_LINE_BYTES,
};
