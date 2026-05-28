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

mod agent;
mod builtins;
#[cfg(feature = "chaos")]
pub mod chaos;
mod error;
mod exec;
mod explore;
pub mod memory;
mod plan;
mod prompt;
mod taskmgmt;
mod tool;
mod web;

pub use agent::{register_agent_tool, SessionFactory};
pub use builtins::{register_builtins, register_builtins_with_executor};
pub use error::{outcome_from_error, ToolError};
pub use exec::{
    executor_for, Isolation, LocalExecutor, SandboxLauncher, SandboxedExecutor, ToolExecutor,
};
pub use explore::{register_explore_tool, report_text, ExploreReport, ExploreStrategy};
#[cfg(any(test, feature = "fake-embed"))]
pub use memory::HashEmbedder;
pub use memory::{
    consolidate_apply, consolidation_prompt, cosine, groom_apply, groom_prompt, recall,
    register_task_tools, AttributionContext, ConsolidationScope, ConsolidationStats, ContextEntry,
    Edge, Embedder, MemType, Memory, Observation, RecallOutput, SqliteStore, SqliteStream, Store,
    Stream, TaskState, TaskStatus, TaskStore, DEFAULT_RECALL_K,
};
pub use plan::register_plan_tools;
pub use prompt::{compaction_prompt, system_prompt, COMPACTION_SYSTEM};
pub use taskmgmt::{register_task_management_tools, BackgroundTaskStore};
pub use tool::{AiSdkTool, ToolFn, ToolRegistry};
pub use web::register_web_tools;
