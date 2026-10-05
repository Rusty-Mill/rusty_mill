//! AG-UI, the Agent-User Interaction protocol, hand-rolled on `rusty_json`.
//!
//! AG-UI is an event stream from an agent to a user interface: a run is
//! opened by `RUN_STARTED`, carries streamed text, tool calls, state
//! snapshots and RFC 6902 state deltas, and is closed by `RUN_FINISHED`
//! or `RUN_ERROR`. The client sends a [`RunAgentInput`] (thread, run,
//! messages, client-side tools, context, state) and reads events back,
//! normally as `text/event-stream`. CopilotKit's React SDK and OpenBot
//! both speak it, so a server that emits it needs no frontend of its own.
//!
//! Layers, each usable alone:
//!
//! - [`types`]: [`RunAgentInput`], [`Message`], [`Tool`], [`Context`].
//! - [`event`]: [`Event`] and [`EventKind`], the 31 event types.
//! - [`codec`]: `Value` in and out for every type, no serde.
//! - [`sse`]: encode events as SSE frames; decode frames incrementally.
//! - [`verify`]: [`Verifier`], the ordering rules and chunk expansion.
//! - [`reduce`]: [`Reducer`], events folded into messages and state.
//! - [`serve`] (feature `serve`): an [`Agent`] trait and a `rusty_serve`
//!   handler that frames a run, verifies what the agent emits, and
//!   streams it.
//!
//! Wire names follow the TypeScript SDK (`RUN_STARTED`, `messageId`).
//! Unknown JSON members are ignored on decode so newer peers still parse.

#![forbid(unsafe_code)]

pub mod codec;
pub mod event;
pub mod reduce;
#[cfg(feature = "serve")]
pub mod serve;
pub mod sse;
pub mod types;
pub mod verify;

mod error;

pub use error::Error;
pub use event::{Event, EventKind, EventMeta, RunOutcome};
pub use reduce::Reducer;
#[cfg(feature = "serve")]
pub use serve::{Agent, AgentHandler, Emitter};
pub use types::{
    Content, ContentPart, Context, FunctionCall, Message, Role, RunAgentInput, Tool, ToolCall,
};
pub use verify::Verifier;

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;
