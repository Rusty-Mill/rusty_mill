//! Pure domain logic for `sessionmgr`.
//!
//! This crate has **zero I/O**. It never opens a file, spawns a process,
//! touches a socket, reads the clock, or generates randomness. Everything
//! that needs any of those is expressed as a value passed in by a caller
//! (see [`SessionId::new`], which takes the clock reading and the random
//! bits rather than sourcing them) or as a port trait an adapter crate
//! implements ([`ports`]).
//!
//! That constraint is what makes the two things most likely to be wrong
//! in this project -- the session state machine and the crash-recovery
//! policy -- fast and deterministic to test, with no real git, no real
//! processes, and no filesystem. See `docs/plan/PLAN.md` § Testing
//! strategy.
//!
//! # Phase scope
//!
//! Phase 1 (walking skeleton) deliberately models only
//! [`SessionKind::PlainTerminal`]. `SameDirectory` and `Worktree` arrive
//! in Phase 2 along with the worktree lifecycle they need, rather than
//! being stubbed in ahead of the code that gives them meaning.

pub mod ports;
pub mod recovery;
pub mod session;

pub use recovery::{decide_recovery, Liveness, RecoveryAction};
pub use session::{
    Session, SessionId, SessionIdError, SessionKind, SessionStatus, TransitionError, WorkerRef,
};
