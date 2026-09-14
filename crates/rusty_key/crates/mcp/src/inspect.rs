//! Tool-return inspection seam (PRD 07 / threat-model). Moved to `constrain`
//! (round7 #25) so `feed::GuideLoader` can reuse the same classifier for
//! `AGENT_GUIDE.md` content without `feed` depending back on `mcp` (`mcp`
//! already depends on `feed`, so the reverse edge would be a cycle). Re-export
//! kept here so this crate's public API (`rk_mcp::{DefaultInspector,
//! Inspection, ReturnInspector}`) and internal `crate::inspect::*` call sites
//! (`manager.rs`, `tool.rs`) are unaffected by the move.

pub use rk_constrain::{DefaultInspector, Inspection, ReturnInspector};
