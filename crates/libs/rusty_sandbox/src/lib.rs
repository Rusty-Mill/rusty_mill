//! Fail-closed sandboxed execution for untrusted processes.
//!
//! A [`SandboxSpec`] says where a process may read and write, what
//! environment it gets and what [`Limits`] bind it; an [`Executor`] runs
//! one program under a spec and reports an [`ExecOutcome`]. The one
//! adapter, [`ProcessExecutor`], spawns a helper binary that confines
//! itself on Linux (Landlock for the filesystem, seccomp for sockets, the
//! process group and `io_uring`, rlimits for CPU, memory, files and
//! processes), verifies every step was enforced, and only then `exec`s the
//! program; a step that cannot be enforced is an [`Error::Sandbox`] and
//! nothing runs. Off Linux every run is refused the same way.
//!
//! The helper is any single-threaded binary that passes its arguments to
//! [`run_helper`]; `rsi __sandbox` is one. Hoisted from `rusty_rsi`
//! (ADR-0005 §4) in ADR-0007 follow-ons step 7, so that a bot can be
//! confined the way an untrusted task solution is.

pub mod error;
pub mod executor;
pub mod helper;
pub mod spec;

pub use error::Error;
pub use executor::{ProcessExecutor, DEFAULT_CAPTURE_BYTES};
#[cfg(target_os = "linux")]
pub use helper::require_enforced;
pub use helper::{run_helper, HelperRequest, Sockets, SETUP_FAILED};
pub use spec::{ExecOutcome, Executor, Limits, SandboxSpec, Termination};
