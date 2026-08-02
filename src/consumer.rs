//! Per-consumer offset tracking, single-node only — no consumer-group
//! rebalancing protocol yet (`docs/phase1-scope.md` §2: that's a Phase 2+
//! problem once there's a second real consumer needing it).
//!
//! Not implemented in this scaffold: a consumer identity plus its last-read
//! [`crate::Offset`] per segment/log, and where that gets persisted (a
//! natural fit for [`crate::Segment`] itself, or a small dedicated
//! offsets file — an open question this scaffold deliberately leaves open
//! rather than guessing).
