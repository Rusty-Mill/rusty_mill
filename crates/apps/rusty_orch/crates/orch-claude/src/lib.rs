//! The `claude` provider over the Claude Code CLI, read-only (ADR-0012).
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
//!
//! [`ClaudeAgent`] also serves the lens runner's [`orch_lens::Model`]
//! (#594): the question's prefix is the system prompt, its body goes on
//! stdin, and the reply text must pass [`orch_lens::check`]. A lens
//! question runs with no tools unless [`ClaudeAgent::lens_read_only`],
//! and only with the operator's no-overage assertion
//! ([`ClaudeAgent::lens_no_overage`]) and a subscription login
//! ([`SUBSCRIPTION_AUTH`]): `claude -p` cannot bound an agent loop's cost
//! before the call, and a login does not prove the account cannot be billed.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod agent;
mod lens;
mod prompt;

pub use agent::{
    ClaudeAgent, AUTH_TIMEOUT, DEFAULT_MAX_TURNS, DEFAULT_TIMEOUT, ID, SCRUBBED_ENV, TOOLS,
};
pub use lens::SUBSCRIPTION_AUTH;
pub use prompt::render;
