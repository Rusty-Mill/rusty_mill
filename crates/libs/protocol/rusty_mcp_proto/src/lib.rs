//! MCP wire types, hand-rolled on `rusty_json`.
//!
//! The Model Context Protocol is JSON-RPC 2.0 plus a fixed vocabulary of
//! methods and payloads. This crate is that vocabulary as plain Rust types
//! with a hand-written codec ([`Wire`]): no serde, no transport, no async.
//! Servers and clients on stdio or Streamable HTTP are built on top of it.
//!
//! This first slice covers the envelope and the tools path, enough for a
//! server or client to list and call tools:
//!
//! - [`rpc`]: [`Message`], [`RequestId`], [`ErrorData`], [`ErrorCode`].
//! - [`version`]: [`ProtocolVersion`] (both the classic and the stateless
//!   2026-07-28 generations) and [`Implementation`].
//! - [`content`]: [`ContentBlock`]; [`resource`]: [`Resource`] and
//!   [`ResourceContents`].
//! - [`page`]: [`PaginatedParams`], [`Paging`], [`CacheScope`].
//! - [`tool`]: [`Tool`], [`ListToolsResult`], [`CallToolParams`],
//!   [`CallToolResult`].
//!
//! Open-ended members (`_meta`, annotations, icons, schemas, error data) are
//! kept as raw [`rusty_json::Value`] so a gateway forwards them untouched.
//! Unknown members of known types are dropped on decode.
//!
//! ```
//! use rusty_mcp_proto::{CallToolParams, Message, RequestId, Wire};
//!
//! let call = Message::request(RequestId::Number(1), "tools/call", &CallToolParams::new("add"));
//! let back = Message::from_json(&call.to_json()).unwrap();
//! assert_eq!(call, back);
//! ```
//!
//! Every type is checked against `rmcp` 3.1's serde model in
//! `tests/oracle.rs`, which is a dev-dependency only.

#![forbid(unsafe_code)]

pub mod codec;
pub mod content;
pub mod page;
pub mod resource;
pub mod rpc;
pub mod tool;
pub mod version;

mod error;

pub use codec::Wire;
pub use content::{ContentBlock, Extras};
pub use error::Error;
pub use page::{CacheScope, PaginatedParams, Paging, ResultType};
pub use resource::{Resource, ResourceContents};
pub use rpc::{ErrorCode, ErrorData, Message, RequestId};
pub use tool::{CallToolParams, CallToolResult, ListToolsResult, Tool};
pub use version::{Implementation, ProtocolVersion};

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;
