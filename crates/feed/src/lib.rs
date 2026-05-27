#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `feed` — the Observe + Orient half of OODA: the tool registry and (later)
//! memory + context assembly. Depends on `config`, `observe`, `constrain`
//! (ARCHITECTURE §4-5).
//!
//! Phase 1 scope: the [`ToolFn`] seam, the [`AiSdkTool`] adapter, the
//! [`ToolRegistry`] (which implements `constrain::ToolDispatch`), and the
//! built-in filesystem tools.

mod builtins;
#[cfg(feature = "chaos")]
pub mod chaos;
mod error;
pub mod memory;
mod prompt;
mod tool;

pub use builtins::register_builtins;
pub use error::{outcome_from_error, ToolError};
pub use memory::{Edge, MemType, Memory, Observation, SqliteStore, SqliteStream, Store, Stream};
pub use prompt::system_prompt;
pub use tool::{AiSdkTool, ToolFn, ToolRegistry};
