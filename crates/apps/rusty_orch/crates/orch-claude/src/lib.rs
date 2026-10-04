//! `Agent::Claude` over the Claude Code CLI, read-only (ADR-0012).
//!
//! A thin adapter on [`orch_cli`]. This crate adds the fixed `claude -p`
//! argv: print mode, a JSON result envelope, the reply constrained to the
//! shared [`orch_cli::output_schema`], the built-in tool set cut to
//! `Read,Grep,Glob` under `--restricted`, every permission prompt denied,
//! no settings files, MCP servers, skills, or session files. It adds the
//! footer, a login probe before each run, and the mapping of the envelope
//! and stderr to classified failures so "not logged in" is an unavailable
//! prerequisite while "rate limited" remains transient.
//!
//! Claude Code authenticates with the Claude subscription login. The child
//! never sees `ANTHROPIC_API_KEY` or a provider override: [`SCRUBBED_ENV`]
//! is removed from the environment on every run.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod agent;
mod prompt;

pub use agent::{
    ClaudeAgent, AUTH_TIMEOUT, DEFAULT_MAX_TURNS, DEFAULT_TIMEOUT, SCRUBBED_ENV, TOOLS,
};
pub use prompt::render;
