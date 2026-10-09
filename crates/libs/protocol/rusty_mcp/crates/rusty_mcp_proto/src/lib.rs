//! The Model Context Protocol wire format, hand-rolled on `rusty_json`.
//!
//! This is the first-party replacement for the `rmcp` model types (see
//! `docs/research/MCP-NATIVE-PLAN.md`). It carries no serde and no async: it
//! turns bytes into typed messages and back, and nothing else.
//!
//! - [`jsonrpc`]: [`Message`], [`RequestId`], [`ErrorObject`], error codes.
//! - [`types`]: `initialize`, tools (`tools/list`, `tools/call`), content.
//! - [`schema`]: [`Schema`], a builder for a tool's `inputSchema`.
//!
//! Encoding emits only the members the reference encoders emit; decoding
//! ignores members it does not know, so a newer peer still parses. Resources,
//! prompts, completion and the 2026-07-28 additions are not here yet; see
//! `docs/research/MCP-PROTO-SCOPE.md` for the build order.

#![forbid(unsafe_code)]

mod error;
pub mod jsonrpc;
pub mod schema;
pub mod types;
mod util;

pub use error::{Error, Result};
pub use jsonrpc::{ErrorObject, Message, RequestId};
pub use schema::Schema;
pub use types::{
    CallToolParams, CallToolResult, Content, Implementation, InitializeParams, InitializeResult,
    ListCapability, ListParams, ListToolsResult, ProtocolVersion, ServerCapabilities, Tool,
};
