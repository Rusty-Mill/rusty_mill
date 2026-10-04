//! Shared core for rusty_orch's CLI agent adapters (ADR-0004).
//!
//! Every adapter is three pure functions and a thin shell. This crate holds
//! the parts that are the same for all of them:
//! - [`CommandRunner`] and [`StdCommand`]: fixed argv, bytes on stdin, a hard
//!   deadline, process-group kill, bounded pipe join, env scrubbing.
//! - [`parse`]: the strict one-JSON-object reply protocol. The kinds a role
//!   may write depend on the goal's `StopRule` (ADR-0011).
//! - [`render`]: the prompt core, with the adapter's format-spec footer
//!   appended.
//! - [`output_schema`]: the JSON Schema of a reply, for CLIs that constrain
//!   their final message to one (Codex, Claude).
//!
//! What stays per adapter: argv, the footer wording, where the reply is read
//! from, and the stderr-to-`AgentError` mapping.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod exec;
#[cfg(feature = "fake")]
pub mod fake;
mod parse;
mod prompt;
mod schema;

pub use exec::{
    excerpt, CommandRunner, ExecError, Exit, StdCommand, EXCERPT_CHARS, JOIN_GRACE,
    MAX_STDOUT_BYTES,
};
pub use parse::{allowed_kinds, parse, parse_ref, MAX_BODY_CHARS, MAX_ENTRIES};
pub use prompt::{format_spec, render, BEST_EFFORT_RULES};
pub use schema::output_schema;
