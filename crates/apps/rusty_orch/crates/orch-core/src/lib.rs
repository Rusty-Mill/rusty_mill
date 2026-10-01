//! Orchestrator domain: goal contracts, task cards, and the shared blackboard.
//!
//! Pure domain logic with no I/O and no dependencies. Adapters (CLI runners,
//! MCP store, persistence) live outside this crate and depend on it.
//!
//! - [`goal`]: validated six-field goal contracts.
//! - [`task`]: task cards and the [`task::Plan`] that owns their lifecycle.
//! - [`board`]: the append-only blackboard agents read and write.

pub mod board;
pub mod goal;
mod ids;
mod reference;
pub mod task;
mod text;

pub use ids::{EntryId, GoalId, TaskId};
pub use reference::Ref;
pub use text::Text;
