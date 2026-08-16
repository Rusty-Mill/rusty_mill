//! The ports: traits the domain defines and adapter crates implement.
//!
//! Only ports Phase 1 actually has an implementation *and* a caller for
//! live here. `GitPort` (Phase 2) and `AgentAdapterPort` (Phase 3) are
//! named in PLAN.md but deliberately not declared yet -- a trait with no
//! implementor is a guess about an interface, and PLAN.md is explicit
//! that the agent-adapter interface in particular depends on the outcome
//! of spikes that have not run.

use crate::session::WorkerRef;

/// Everything the domain needs from OS process management.
///
/// Implemented by `sessionmgr-proc` against real syscalls, and by fakes
/// in tests.
///
/// Note what is **not** here: any notion of a process group, job, or
/// tree-kill. Windows Job Objects are kill-on-close, which is structurally
/// incompatible with a session surviving the manager exiting -- so the
/// port offers per-pid operations only, and teardown targets an explicit
/// pid list (see [`crate::recovery::teardown_pids`]).
pub trait ProcessPort {
    /// Is `pid` alive **and** still the same process that recorded
    /// `expected`?
    ///
    /// The two-part question matters: a bare liveness check answers "does
    /// some process hold this number", which after pid reuse is a
    /// different question with the same answer. A supervisor that trusts
    /// the bare check declines to mark a genuinely dead worker as
    /// crashed, leaving the session wedged with nothing running and
    /// nothing noticing.
    fn is_same_process(&self, pid: u32, expected: Option<&str>) -> bool;

    /// An opaque, platform-specific fingerprint of when `pid` started.
    /// `None` when this platform cannot supply one.
    fn start_fingerprint(&self, pid: u32) -> Option<String>;

    /// Terminates `pid`. Best-effort: a pid that is already gone is not
    /// an error, since the caller's goal is "not running", which is
    /// already true.
    fn terminate(&self, pid: u32) -> std::io::Result<()>;
}

/// Convenience: build a [`WorkerRef`] for a pid this process just
/// spawned, capturing its fingerprint immediately.
///
/// Immediately, and not later, is the point: the fingerprint is only
/// meaningful if it is taken while the recorded process is still
/// definitely the one that was spawned. Reading it back at recovery time
/// would fingerprint whatever holds the pid *then*, which is precisely
/// the confusion it exists to prevent.
pub fn worker_ref(port: &dyn ProcessPort, pid: u32) -> WorkerRef {
    WorkerRef {
        pid,
        start_fingerprint: port.start_fingerprint(pid),
    }
}
