//! The sandboxed-execution port (ADR-0005 §4, invariant 5).
//!
//! Every untrusted process (a task solution, the inner harness and its
//! build, a coding-agent CLI) runs through an [`Executor`] under a
//! [`SandboxSpec`]. The types moved to `rusty_sandbox` in ADR-0007
//! follow-ons step 7 and are re-exported here unchanged; a spec that
//! fails validation converts to a [`crate::CoreError::InvalidId`].

pub use rusty_sandbox::{ExecOutcome, Executor, Limits, SandboxSpec, Termination};
