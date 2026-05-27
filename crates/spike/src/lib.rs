//! # rk-spike — de-risking the aisdk ↔ harness seam
//!
//! A throwaway spike (BACKLOG Phase 1 risk: "aisdk `#[tool]`→`ToolFn` adapter is
//! the riskiest seam → spike it first"). It proves, against aisdk 0.5.2:
//!
//! - the `#[tool]` macro can feed our async [`tool::ToolFn`] adapter (schema
//!   reused, sync `execute` bypassed);
//! - policy vets every call before dispatch ([`tool::ToolRegistry`]);
//! - the kernel drives its own loop ([`kernel::run_turn`]) because aisdk's
//!   built-in loop is not interceptable;
//! - a live aisdk model adapts to the kernel's [`kernel::ChatModel`] port
//!   ([`aisdk_adapter`]) with no fork.

pub mod aisdk_adapter;
pub mod error;
pub mod fake;
pub mod kernel;
pub mod outcome;
pub mod policy;
pub mod tool;
