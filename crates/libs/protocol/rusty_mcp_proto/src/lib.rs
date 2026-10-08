//! MCP wire types, hand-rolled on `rusty_json`.
//!
//! The Model Context Protocol is JSON-RPC 2.0 plus a fixed vocabulary of
//! methods and payloads. This crate is that vocabulary as plain Rust types
//! with a hand-written codec ([`Wire`]): no serde, no transport, no async.
//! Servers and clients on stdio or Streamable HTTP are built on top of it.
//!
//! Covered: the envelope, both handshakes, capabilities, tools, prompts,
//! resources, completion, cancel/progress, subscriptions, tasks and
//! multi-round-trip input:
//!
//! - [`rpc`]: [`Message`], [`RequestId`], [`ErrorData`], [`ErrorCode`].
//! - [`version`]: [`ProtocolVersion`] (both the classic and the stateless
//!   2026-07-28 generations) and [`Implementation`].
//! - [`content`]: [`ContentBlock`]; [`resource`]: [`Resource`] and
//!   [`ResourceContents`].
//! - [`page`]: [`PaginatedParams`], [`Paging`], [`CacheScope`].
//! - [`tool`]: [`Tool`], [`ListToolsResult`], [`CallToolParams`],
//!   [`CallToolResult`].
//! - [`prompt`]: [`Prompt`], [`GetPromptParams`], [`GetPromptResult`], [`Role`].
//! - [`resource`] also has templates and the list/read params and results.
//! - [`completion`]: [`Reference`], [`CompleteParams`], [`CompleteResult`].
//! - [`capabilities`], [`lifecycle`], [`meta`]: [`ServerCapabilities`],
//!   [`ClientCapabilities`], [`InitializeParams`]/[`InitializeResult`] (classic),
//!   [`DiscoverResult`] (stateless) and [`RequestMeta`], the typed per-request
//!   `_meta` of the stateless protocol.
//! - [`notify`]: [`CancelledParams`], [`ProgressParams`].
//! - [`subscribe`]: [`SubscriptionFilter`], [`ListenParams`], [`ListenResult`].
//! - [`task`]: [`Task`], [`DetailedTask`], [`CreateTaskResult`], [`GetTaskResult`].
//! - [`input`]: [`InputRequiredResult`], [`CallToolResponse`], [`Outcome`],
//!   [`ElicitParams`], [`ElicitResult`].
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

pub mod capabilities;
pub mod codec;
pub mod completion;
pub mod content;
pub mod input;
pub mod lifecycle;
pub mod meta;
pub mod notify;
pub mod page;
pub mod prompt;
pub mod resource;
pub mod rpc;
pub mod subscribe;
pub mod task;
pub mod tool;
pub mod version;

mod error;

pub use capabilities::{
    ClientCapabilities, PromptsCapability, ResourcesCapability, ServerCapabilities, ToolsCapability,
};
pub use codec::Wire;
pub use completion::{ArgumentInfo, CompleteParams, CompleteResult, CompletionInfo, Reference};
pub use content::{ContentBlock, Extras};
pub use error::Error;
pub use input::{
    CallToolResponse, ElicitAction, ElicitParams, ElicitResult, GetPromptResponse, InputRequest,
    InputRequests, InputRequiredResult, Outcome, ReadResourceResponse,
};
pub use lifecycle::{
    falls_back_to_initialize, negotiate_classic, pick_common, DiscoverParams, DiscoverResult,
    InitializeParams, InitializeResult,
};
pub use meta::RequestMeta;
pub use notify::{CancelledParams, ProgressParams, ProgressToken};
pub use page::{CacheScope, PaginatedParams, Paging, ResultType};
pub use prompt::{
    GetPromptParams, GetPromptResult, ListPromptsResult, Prompt, PromptArgument, PromptMessage,
    Role,
};
pub use resource::{
    ListResourceTemplatesResult, ListResourcesResult, ReadResourceParams, ReadResourceResult,
    Resource, ResourceContents, ResourceTemplate,
};
pub use rpc::{ErrorCode, ErrorData, Message, RequestId};
pub use subscribe::{
    AcknowledgedParams, ListenParams, ListenResult, ResourceUpdatedParams, SubscribeParams,
    SubscriptionFilter,
};
pub use task::{
    CreateTaskResult, DetailedTask, GetTaskResult, Task, TaskAckResult, TaskIdParams, TaskPayload,
    TaskStatus, TaskStatusParams, UpdateTaskParams,
};
pub use tool::{CallToolParams, CallToolResult, ListToolsResult, Tool};
pub use version::{Implementation, ProtocolVersion};

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;
