//! `Agent::Local` over the Ollama CLI.
//!
//! A thin adapter on [`orch_cli`]: the process seam, the JSON reply
//! protocol, and the prompt core live there (ADR-0004). This crate adds the
//! argv, the format-spec footer, and the stderr-to-`AgentError` mapping.
//!
//! - [`render`] builds the prompt with Ollama's footer.
//! - [`OllamaAgent`] implements [`AgentRunner`] for [`Agent::Local`] by
//!   piping the prompt into `ollama run <model> --format json` through a
//!   [`CommandRunner`].
//!
//! [`Agent::Local`]: orch_core::task::Agent::Local
//! [`AgentRunner`]: orch_dispatch::AgentRunner
//! [`CommandRunner`]: orch_cli::CommandRunner

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod agent;
mod prompt;

pub use agent::OllamaAgent;
pub use prompt::render;
