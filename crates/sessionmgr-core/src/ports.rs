//! The ports: traits the domain defines and adapter crates implement.
//!
//! Only ports that actually have an implementation *and* a caller live
//! here. `AgentAdapterPort` (Phase 3) is named in the plan but
//! deliberately not declared yet -- a trait with no implementor is a
//! guess about an interface, and the agent-adapter interface in
//! particular depends on the outcome of spikes that have not all run.

use std::path::Path;

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

/// A file changed in a session's workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Two-character porcelain status (`" M"`, `"??"`, `"A "`, …), kept
    /// raw rather than parsed into an enum: git's porcelain v1 codes
    /// carry staged-vs-unstaged in the two positions, and collapsing
    /// that into a friendlier type would lose information the diff view
    /// wants.
    pub status: String,
    pub path: String,
}

/// Everything the domain needs from git.
///
/// Every method takes an explicit path rather than the adapter holding
/// one: a single daemon serves sessions across many repositories at once,
/// so an adapter bound to one repository would need one instance per
/// session and would make "which repo did this operate on" implicit at
/// exactly the moments it matters most.
pub trait GitPort {
    /// Is `path` inside a git repository, and if so where is its root?
    ///
    /// Returns the root rather than a bool because every caller that
    /// wants the answer also wants the root: worktrees are placed
    /// relative to it, and a session created from a subdirectory must
    /// still resolve to the same repository as one created from the top.
    fn repo_root(&self, path: &Path) -> Result<std::path::PathBuf, GitError>;

    /// Creates a worktree at `worktree` on a new branch `branch`.
    fn worktree_add(&self, repo: &Path, worktree: &Path, branch: &str) -> Result<(), GitError>;

    /// Removes a worktree.
    ///
    /// `force` corresponds to `git worktree remove --force`, which is
    /// what discards a worktree with uncommitted changes in it.
    fn worktree_remove(&self, repo: &Path, worktree: &Path, force: bool) -> Result<(), GitError>;

    /// Deletes a branch this tool created.
    fn branch_delete(&self, repo: &Path, branch: &str, force: bool) -> Result<(), GitError>;

    /// Merges `branch` into the repository's current branch,
    /// **fast-forward only**.
    fn merge_fast_forward_only(&self, repo: &Path, branch: &str) -> Result<(), GitError>;

    /// The files changed in `workspace`, staged or not.
    fn changed_files(&self, workspace: &Path) -> Result<Vec<ChangedFile>, GitError>;

    /// A unified diff of `workspace`. `path` narrows it to one file.
    fn diff(&self, workspace: &Path, path: Option<&str>) -> Result<String, GitError>;
}

/// A git operation that failed.
///
/// Carries git's own stderr rather than replacing it with a friendlier
/// message: git's diagnostics for the failures that actually happen here
/// ("cannot remove a locked working tree", "not something we can merge")
/// are better than anything this layer could invent, and hiding them
/// would make the one thing a user needs -- what git objected to --
/// unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    pub operation: &'static str,
    pub message: String,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "git {} failed: {}", self.operation, self.message.trim())
    }
}

impl std::error::Error for GitError {}

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
