//! `Agent::Local` over the Ollama CLI.
//!
//! Three pure functions and one thin I/O shell:
//! - [`render`] turns a task card plus the board entries it references into
//!   the prompt, ending with the output-format spec.
//! - [`parse`] turns the model's reply into [`Output`] entries, strictly.
//! - [`OllamaAgent`] implements [`AgentRunner`] for [`Agent::Local`] by
//!   piping the prompt into `ollama run <model> --format json` through a
//!   [`CommandRunner`].
//!
//! The protocol is recorded in ADR-0004.
//!
//! [`Agent::Local`]: orch_core::task::Agent::Local
//! [`AgentRunner`]: orch_dispatch::AgentRunner
//! [`Output`]: orch_dispatch::Output

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod agent;
mod exec;
mod parse;
mod prompt;

pub use agent::OllamaAgent;
pub use exec::{CommandRunner, ExecError, Exit, StdCommand, JOIN_GRACE, MAX_STDOUT_BYTES};
pub use parse::{allowed_kinds, parse, MAX_BODY_CHARS, MAX_ENTRIES};
pub use prompt::render;
