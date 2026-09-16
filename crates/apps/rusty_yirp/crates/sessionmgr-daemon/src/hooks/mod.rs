//! Session hooks / extensibility (PLAN.md's Phase 4+ section, now
//! wired): (1) installing a CLI's own hook config to call back into
//! `sessionmgr __hook-fire`, and (2) outbound webhook dispatch on the
//! events that result.
//!
//! A module, not a crate, per PLAN.md's own file layout -- this is
//! composition-root logic (it reads the session record, resolves the
//! running binary's own path, writes into a session's workspace), not a
//! reusable adapter.

pub mod dispatch;
pub mod install;
