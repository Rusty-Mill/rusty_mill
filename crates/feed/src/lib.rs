//! `feed` — the Observe + Orient half of OODA: the tool registry and (later)
//! memory + context assembly. Depends on `config`, `observe`, `constrain`
//! (ARCHITECTURE §4-5).
//!
//! Phase 1 scope: the [`ToolFn`] seam, the [`AiSdkTool`] adapter, the
//! [`ToolRegistry`] (which implements `constrain::ToolDispatch`), and the
//! built-in filesystem tools.

mod builtins;
mod error;
mod tool;

pub use builtins::register_builtins;
pub use error::{outcome_from_error, ToolError};
pub use tool::{AiSdkTool, ToolFn, ToolRegistry};
