//! A blocking MCP server scaffold on `rusty_mcp_proto`.
//!
//! Three layers, each usable alone:
//!
//! - [`Dispatcher`]: sans-IO. Give it a decoded [`Message`] and a [`Service`],
//!   get back the response to send, if any. No threads, no I/O.
//! - [`Server`]: a [`Service`] built from registered tools
//!   (`Server::new(..).tool(Tool, handler)`), with `Schema`-built input schemas.
//! - [`stdio`]: newline-delimited JSON over a reader and a writer, with a
//!   bounded line length and `notifications/cancelled` honoured while a
//!   handler runs.
//!
//! Handlers are plain synchronous closures. A handler that wants to stop early
//! polls [`Context::is_cancelled`].

#![forbid(unsafe_code)]

mod context;
mod dispatch;
mod server;
pub mod stdio;

pub use context::Context;
pub use dispatch::{Dispatcher, Service, SUPPORTED_VERSIONS};
pub use server::{Server, ToolError};
