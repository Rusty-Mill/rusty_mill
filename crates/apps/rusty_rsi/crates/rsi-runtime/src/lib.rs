//! Adapters for the `rusty_rsi` ports (ADR-0005).
//!
//! - [`executor`]: [`ProcessExecutor`], the sandboxed [`rsi_core::Executor`].
//! - [`sandbox`]: the helper process that confines itself and execs.
//! - [`task_dir`]: the on-disk task format, [`TaskDir`].
//! - [`metric`]: pure output metrics.
//! - [`grading`]: [`LocalTask`] (public) and [`SandboxedGrader`] (private).

pub mod error;
pub mod executor;
pub mod grading;
pub mod metric;
pub mod sandbox;
pub mod task_dir;

pub use error::RuntimeError;
pub use executor::ProcessExecutor;
pub use grading::{GraderCommand, LocalTask, SandboxedGrader, SolutionRunner};
pub use task_dir::{Split, TaskDir};
