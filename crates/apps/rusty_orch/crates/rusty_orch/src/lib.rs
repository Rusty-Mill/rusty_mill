//! `rusty_orch`: the orchestrator as a command.
//!
//! Reads one JSON goal file (the goal contract, a human-authored task list,
//! optional routing), runs it through `orch-dispatch` over the Codex and
//! Ollama adapters, and prints the plan, the board, and the call ledger.
//! Everything that parses or prints lives here, in the application layer;
//! `orch-core` stays I/O-free ([ADR-0009](../../../docs/adr/0009-command-line.md)).
//!
//! - [`args`]: the command line, parsed over `std::env::args` with no
//!   dependency.
//! - [`input`]: the goal file, parsed with `rusty_json` into `orch-core`
//!   drafts and specs.
//! - [`agents`]: the composite runner over the two real adapters.
//! - [`run`]: the loop around the dispatcher: wall clock, blocked questions,
//!   interactive answers.
//! - [`report`]: text and JSON views of the result.
//! - [`cli`]: the entry point over injectable streams; stdout is the report
//!   only, questions and progress go to stderr.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod agents;
pub mod args;
pub mod cli;
pub mod input;
pub mod report;
pub mod run;
