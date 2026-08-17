//! The ports: traits the domain defines and adapter crates implement.
//!
//! Only ports that actually have an implementation *and* a caller live
//! here. `AgentAdapterPort` was named in the plan but deliberately not
//! declared until Phase 3 -- a trait with no implementor is a guess
//! about an interface. It has two now (`sessionmgr-agents`' `ClaudeCode`
//! and `Codex`), both driven by real, measured CLI output rather than
//! assumed shapes.

use std::path::{Path, PathBuf};

use crate::session::{SessionId, WorkerRef};

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
    ///
    /// `start_point`, when `Some`, is the commit-ish the new branch is
    /// created from -- `None` means git's own default (the currently
    /// checked-out `HEAD` of `repo`'s own working copy), which is what
    /// every ordinary worktree session wants. Fork needs the other case:
    /// a forked session's new worktree must start from **the source
    /// session's own branch tip**, not the repository's main branch, so
    /// the code state it starts working in matches the conversation
    /// history it starts with. Without this, a forked agent would find
    /// itself talking about edits that are not actually on disk.
    fn worktree_add(
        &self,
        repo: &Path,
        worktree: &Path,
        branch: &str,
        start_point: Option<&str>,
    ) -> Result<(), GitError>;

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
/// `needs_input` is PLAN.md's "hard part" -- tier-3 (pattern-matching).
/// Tier 1 (hooks, `hook_config`/`hook_signal`) is a higher-confidence,
/// out-of-band signal that arrives through a hook callback, not through
/// output text. Tier 2 (process exit) is `Session::record_exit`,
/// already wired for every session regardless of whether it has an
/// adapter at all.
pub trait AgentAdapterPort {
    /// The command line to launch this agent. `extra` is whatever the
    /// caller supplied beyond `--agent <kind>` -- typically empty
    /// (bare interactive) or an initial prompt. `hooks_enabled` is
    /// whether `--hooks` requested a hook install for this session --
    /// some CLIs need their own extra flag to run an installed hook
    /// without an interactive trust-review gate first (measured: Codex
    /// does, `--dangerously-bypass-hook-trust`; Claude Code does not).
    ///
    /// `native_id`, when `Some`, asks this adapter to pin the CLI's own
    /// session identifier to that value at launch, if it supports doing
    /// so (see [`Self::fork_args`]'s own docs for which adapters do, and
    /// why this is what makes Fork possible without the daemon having to
    /// discover a session's native id after the fact by scanning the
    /// CLI's own state directory). An adapter that does not support
    /// pinning simply ignores it rather than erroring -- the daemon only
    /// ever passes `Some` when it already knows this adapter supports
    /// it, but an adapter must still behave sensibly if it is passed
    /// anyway, the same defensive posture every other method on this
    /// trait already takes toward its inputs.
    fn launch_args(
        &self,
        extra: &[String],
        hooks_enabled: bool,
        native_id: Option<&str>,
    ) -> Vec<String>;

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
    /// confidence badge.
    fn has_verified_hooks(&self) -> bool;

    /// The relative path (under the session's own workspace directory)
    /// and file content for this CLI's own hook configuration format,
    /// pointing every event this adapter cares about at
    /// `hook_fire_exe __hook-fire --session-id <id> --event <name>`,
    /// invoked directly (no shell, no script file).
    ///
    /// Measured, not assumed (`docs/phase-4-hooks-report.md`): a real
    /// Claude Code hook `command` string is tokenized with POSIX-style
    /// backslash-escaping even on Windows, so a raw Windows path with
    /// single backslashes silently loses them (`C:\a\b.exe` becomes
    /// `C:ab.exe`). `hook_fire_exe` must already be forward-slashed by
    /// the caller; this method does not do it, since that is a Windows-
    /// path concern, not a per-CLI one.
    ///
    /// Pure -- builds a path and a string, touches no filesystem. The
    /// actual write is `sessionmgr-daemon::hooks::install`'s job: this
    /// crate is zero-I/O by design.
    fn hook_config(&self, hook_fire_exe: &Path, session_id: &SessionId) -> (PathBuf, String);

    /// What installing a hook and having it fire for `event` means.
    fn hook_signal(&self, event: &str) -> HookOutcome;

    /// Does this adapter support Fork at all?
    ///
    /// Answers a **creation-time** question [`Self::fork_args`] cannot:
    /// whether it is worth generating and pinning a native session id
    /// *before* anyone has asked to fork anything, so an ordinary session
    /// is already forkable later with no further machinery. Kept as its
    /// own explicit fact, deliberately, rather than inferred by probing
    /// `fork_args` with placeholder ids -- the same "answer the question,
    /// do not infer it" reasoning [`Self::has_verified_hooks`] already
    /// follows. The two methods are two questions asked at two different
    /// times, not one capability duplicated: `sessionmgr-agents`' own
    /// tests assert `supports_fork() == fork_args(..).is_some()` for
    /// every adapter, which is what keeps them from drifting apart in
    /// practice.
    ///
    /// As of ADR-0003/Phase 6, only Claude Code answers `true`: it is the
    /// one CLI of the three this project supports that could actually be
    /// live-verified in the environment available while this was built
    /// (real, authenticated `claude` access), not just reasoned about
    /// from source. See [`Self::fork_args`]'s own docs for why Codex and
    /// Gemini CLI are not `true` here yet, and what would change that.
    fn supports_fork(&self) -> bool;

    /// The command line to launch a **forked** session -- a new,
    /// independent session that starts with a copy of `source_native_id`'s
    /// own conversation history, per CAPABILITIES.md's observed "Fork
    /// session" capability and ADR-0003's spike into whether any CLI
    /// supports being handed externally-tracked prior state at all.
    ///
    /// `None` when [`Self::supports_fork`] is `false` for this adapter.
    /// Codex's own fork mechanism is real (`codex fork <id>`, confirmed
    /// via its own test suite in ADR-0003) but needs a *separate*
    /// native-id-**discovery** mechanism this phase does not build, since
    /// unlike Claude Code Codex has no flag to let the caller pin a new
    /// session's id at creation -- see `docs/phase-6-report.md` for
    /// exactly what is missing and why this was not guessed at instead.
    /// Gemini CLI's own fork-equivalent (`--session-file`) needs the
    /// *file path* to the source session's own chat history, which
    /// requires replicating gemini-cli's internal project-directory
    /// hashing scheme to locate reliably -- also deferred, also explained
    /// in the phase report rather than shipped unverified.
    ///
    /// `new_native_id` is the id the *forked* session should itself be
    /// pinned to (see [`Self::launch_args`]'s own `native_id` parameter),
    /// so a forked session is just as forkable again afterward as an
    /// ordinary one.
    fn fork_args(
        &self,
        source_native_id: &str,
        new_native_id: &str,
        extra: &[String],
    ) -> Option<Vec<String>>;
}

/// [`AgentAdapterPort::needs_input`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSignal {
    Running,
    NeedsInput,
}

/// [`AgentAdapterPort::hook_signal`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOutcome {
    /// Transition the session to this signal, the same way a tier-3
    /// pattern match would -- both ultimately call the same transition
    /// path, so a hook and a pattern match agreeing is not a conflict,
    /// just redundant confirmation.
    Status(AgentSignal),
    /// No status change, but worth telling the webhook dispatcher about
    /// (a sub-agent finishing, say -- PLAN.md's `SubagentFinished`).
    Notify,
    /// Not an event this adapter maps to anything.
    Ignore,
}
