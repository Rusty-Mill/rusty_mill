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
pub mod ports;
pub mod recovery;
pub mod session;
pub mod workspace;

pub use recovery::{decide_recovery, Liveness, RecoveryAction};
pub use session::{
    AgentKind, Disposition, Session, SessionId, SessionIdError, SessionKind, SessionStatus,
    TransitionError, WorkerRef,
};
pub use workspace::Workspace;
