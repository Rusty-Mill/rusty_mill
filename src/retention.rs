//! Segment rolling and retention — size/time-based, no compaction yet
//! (`docs/phase1-scope.md` §2).
//!
//! Not implemented in this scaffold: rolling a [`crate::Segment`] once it
//! crosses a configured size or age, and deleting retired segments once
//! they age out. Both operations are straightforward given
//! [`crate::segment::Segment`]'s existing primitives (`rusty_tokio::io`'s
//! `uring_rename`/`uring_remove_file` for the actual roll/delete, matching
//! `docs/adr/0002-phase1-foundational-decisions.md`'s D3), but the actual
//! policy (what "size-based" and "time-based" mean numerically, how
//! multiple segments are tracked as one logical log) is real design work
//! that shouldn't be improvised into existence alongside the initial
//! scaffold — see `docs/phase1-scope.md` §2 for the scope this needs to
//! satisfy when it's built.
