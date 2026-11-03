//! The `codex` provider over the Codex CLI, read-only (ADR-0006).
//!
//! A thin adapter on [`orch_cli`]. This crate adds the fixed `codex exec`
//! argv, the read-only sandbox, the shared output schema and last-message
//! file that give one clean JSON object back, the footer, and the mapping of
//! Codex's stderr to classified failures so "not logged in" is an unavailable
//! prerequisite while "rate limited" remains transient.
//!
//! Codex authenticates with the ChatGPT subscription login. The child never
//! sees `OPENAI_API_KEY`, `CODEX_API_KEY` or `OPENAI_BASE_URL`: they are
//! scrubbed from the environment on every run.
//!
//! [`CodexAgent`] also serves the lens runner's [`orch_lens::Model`]
//! (#594), on a ChatGPT subscription login only ([`LOGIN_ARGV`]): prefix
//! and body on stdin, measured usage from the `--json` events, the reply
//! checked by [`orch_lens::parse`]. `codex exec` cannot bound its cost
//! before the call, so it is never priced per token. A lens question runs in an empty
//! scratch directory unless [`CodexAgent::lens_read_only`]; the read-only
//! sandbox itself is unchanged.
//!
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod agent;
mod lens;
mod prompt;

pub use agent::{CodexAgent, DEFAULT_TIMEOUT, ID, SCRUBBED_ENV};
pub use lens::{LOGIN_ARGV, LOGIN_TIMEOUT};
/// The reply schema handed to `--output-schema`; shared with the Claude
/// adapter and kept in `orch-cli`.
pub use orch_cli::output_schema;
pub use prompt::render;
