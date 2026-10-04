//! Adapters for the `rusty_rsi` ports (ADR-0005).
//!
//! - [`executor`]: [`ProcessExecutor`], the sandboxed [`rsi_core::Executor`].
//! - [`sandbox`]: the helper process that confines itself and execs.
//! - [`task_dir`]: the on-disk task format, [`TaskDir`].
//! - [`metric`]: pure output metrics.
//! - [`output`]: safe ingestion of files a sandboxed program left behind.
//! - [`grading`]: [`LocalTask`] (public) and [`SandboxedGrader`] (private).
//! - [`protocol`]: the broker wire format shared with the inner agent.
//! - [`broker`]: the metered service the inner agent talks to.
//! - [`model`]: chat-model clients, live ([`OpenAiModel`]) and scripted.
//! - [`harness`]: building the agent and running it against the broker.
//! - [`git`]: sparse candidate worktrees and the path allowlist.
//! - [`lineage_store`]: the hash-chained run record and its blobs.
//! - [`proposer`]: harness rewrites by a chat model, or scripted.
//! - [`outer`]: the outer loop and noise calibration.
//! - [`report`]: a run's summary, and its grade and trajectory replay.

pub mod broker;
pub mod error;
pub mod executor;
pub mod git;
pub mod grading;
pub mod harness;
pub mod lineage_store;
pub mod metric;
pub mod model;
pub mod outer;
pub mod output;
pub mod proposer;
pub mod protocol;
pub mod report;
pub mod sandbox;
pub mod task_dir;

pub use error::RuntimeError;
pub use executor::ProcessExecutor;
pub use grading::{GraderCommand, LocalTask, SandboxedGrader, SolutionRunner};
pub use harness::{HarnessProcess, SandboxedHarness, Toolchain};
pub use model::{OpenAiModel, ScriptedModel};
pub use task_dir::{Split, TaskDir};
