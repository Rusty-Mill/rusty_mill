//! The Model Context Protocol wire format, hand-rolled on `rusty_json`.
//!
//! This is the first-party replacement for the `rmcp` model types (see
//! `docs/research/MCP-NATIVE-PLAN.md`). It carries no serde and no async: it
//! turns bytes into typed messages and back, and nothing else.
//!
//! - [`jsonrpc`]: [`Message`], [`RequestId`], [`ErrorObject`], error codes.
//! - [`types`]: `initialize`, tools (`tools/list`, `tools/call`), content.
//! - [`resources`], [`prompts`], [`completion`]: the matching methods.
//! - [`schema`]: [`Schema`], a builder for a tool's `inputSchema`.
//!
//! Encoding emits only the members the reference encoders emit; decoding
//! ignores members it does not know, so a newer peer still parses. Tools,
//! resources, prompts and completion are here; tasks, elicitation and the
//! 2026-07-28 additions are not yet; see
//! `docs/research/MCP-PROTO-SCOPE.md` for the build order.

#![forbid(unsafe_code)]

pub mod completion;
mod error;
pub mod jsonrpc;
pub mod prompts;
pub mod resources;
pub mod schema;
pub mod types;
mod util;

pub use completion::{ArgumentInfo, CompleteParams, CompleteResult, Completion, Reference};
pub use error::{Error, Result};
pub use jsonrpc::{ErrorObject, Message, RequestId};
pub use prompts::{
    GetPromptParams, GetPromptResult, ListPromptsResult, Prompt, PromptArgument, PromptMessage,
    Role,
};
pub use resources::{
    ListResourceTemplatesResult, ListResourcesResult, ReadResourceParams, ReadResourceResult,
    Resource, ResourceContents, ResourceTemplate,
};
pub use schema::Schema;
pub use types::{
    CallToolParams, CallToolResult, Content, Implementation, InitializeParams, InitializeResult,
    ListCapability, ListParams, ListToolsResult, ProtocolVersion, ResourcesCapability,
    ServerCapabilities, Tool,
};
