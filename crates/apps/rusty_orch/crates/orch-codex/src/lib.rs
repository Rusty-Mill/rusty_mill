//! `Agent::Codex` over the Codex CLI, read-only (ADR-0006).
//!
//! A thin adapter on [`orch_cli`]. This crate adds the fixed `codex exec`
//! argv, the read-only sandbox, the output schema and last-message file
//! that give one clean JSON object back, the footer, and the mapping of
//! Codex's stderr to typed [`AgentError`]s so "not logged in" and "rate
//! limited" read differently.
//!
//! Codex authenticates with the ChatGPT subscription login. The child never
//! sees `OPENAI_API_KEY`: it is scrubbed from the environment on every run.
//!
//! [`AgentError`]: orch_dispatch::AgentError

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod agent;
mod prompt;
mod schema;

pub use agent::{CodexAgent, DEFAULT_TIMEOUT, SCRUBBED_ENV};
pub use prompt::render;
pub use schema::OUTPUT_SCHEMA;
