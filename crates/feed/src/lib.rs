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
#[cfg(any(test, feature = "fake-embed"))]
pub use memory::HashEmbedder;
pub use memory::{
    consolidate_apply, consolidation_prompt, cosine, groom_apply, groom_prompt, recall,
    register_task_tools, AttributionContext, ConsolidationScope, ConsolidationStats, ContextEntry,
    Edge, Embedder, MemType, Memory, Observation, RecallOutput, SqliteStore, SqliteStream, Store,
    Stream, TaskState, TaskStatus, TaskStore, DEFAULT_RECALL_K,
};
pub use prompt::system_prompt;
pub use tool::{AiSdkTool, ToolFn, ToolRegistry};
