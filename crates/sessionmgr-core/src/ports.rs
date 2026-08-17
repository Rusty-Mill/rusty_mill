//! The ports: traits the domain defines and adapter crates implement.
//!
//! Only ports that actually have an implementation *and* a caller live
//! here. `AgentAdapterPort` was named in the plan but deliberately not
//! declared until Phase 3 -- a trait with no implementor is a guess
//! about an interface. It has two now (`sessionmgr-agents`' `ClaudeCode`
//! and `Codex`), both driven by real, measured CLI output rather than
//! assumed shapes.

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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Everything the domain needs from a per-CLI agent adapter.
///
/// Implemented by `sessionmgr-agents`. `launch_args` is the easy half;
/// `needs_input` is PLAN.md's "hard part" -- tier-3 (pattern-matching)
/// only, per this trait. Tier 1 (hooks) is a higher-confidence,
/// out-of-band signal that arrives through a hook callback, not through
/// output text, so it is not this trait's concern -- it updates a
/// session's status directly once Phase 4's hook-install work exists.
/// Tier 2 (process exit) is `Session::record_exit`, already wired for
/// every session regardless of whether it has an adapter at all.
pub trait AgentAdapterPort {
    /// The command line to launch this agent. `extra` is whatever the
    /// caller supplied beyond `--agent <kind>` -- typically empty
    /// (bare interactive) or an initial prompt.
    fn launch_args(&self, extra: &[String]) -> Vec<String>;

    /// Does the CLI's current on-screen state look like it is waiting on
    /// the user? `screen_text` is already-rendered plain text (one
    /// screen row per line, escape sequences already interpreted, ANSI
    /// colors and cursor-positioning gone) -- never raw PTY bytes, which
    /// keeps this trait free of any terminal-emulation dependency.
    /// Measured directly (see `docs/phase-3-report.md`): the naive
    /// alternative, matching against ANSI-stripped raw bytes, silently
    /// fails whenever a CLI lays out spacing with cursor-positioning
    /// sequences instead of literal space characters, which both
    /// adapters this trait has today actually do.
    ///
    /// Only ever answers `Running` or `NeedsInput` -- see the trait's
    /// own docs for why `Finished`/`Errored` are deliberately not this
    /// tier's call.
    fn needs_input(&self, screen_text: &str) -> AgentSignal;

    /// Does this CLI have a hook mechanism sessionmgr has verified
    /// firing for real (not just documented)? Recorded for a future
    /// confidence badge and for Phase 4's hook-install work; this crate
    /// never installs a hook itself.
    fn has_verified_hooks(&self) -> bool;
}

/// [`AgentAdapterPort::needs_input`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSignal {
    Running,
    NeedsInput,
}
