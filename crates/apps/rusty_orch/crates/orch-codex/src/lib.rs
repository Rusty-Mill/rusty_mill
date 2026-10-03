//! `Agent::Codex` over the Codex CLI, read-only (ADR-0006).
//!
//! A thin adapter on [`orch_cli`]. This crate adds the fixed `codex exec`
//! argv, the read-only sandbox, the output schema and last-message file
//! that give one clean JSON object back, the footer, and the mapping of
//! Codex's stderr to classified failures so "not logged in" is an unavailable
//! prerequisite while "rate limited" remains transient.
//!
//! Codex authenticates with the ChatGPT subscription login. The child never
//! sees `OPENAI_API_KEY`: it is scrubbed from the environment on every run.
//!
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
