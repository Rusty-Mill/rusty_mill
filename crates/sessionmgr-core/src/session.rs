//! The session identifier, kind, status, and the state machine that
//! governs which status may follow which.

use serde::{Deserialize, Serialize};

use crate::workspace::Workspace;

/// What to do with a worktree session's branch when tearing it down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    /// Merge the session's branch back into the repository.
    ///
    /// **Fast-forward only.** A session that has diverged from its base
    /// fails loudly rather than being silently three-way merged: this
    /// tool is tearing down a workspace, which is the worst possible
    /// moment to invent a merge commit the user did not ask for and is
    /// not watching.
    Merge,
    /// Throw the session's branch and worktree away.
    Discard,
}

/// Crockford base32 alphabet, minus `I`/`L`/`O`/`U` -- the standard set
/// chosen so a transcribed id can't confuse `1`/`I`/`l` or `0`/`O`, and
/// so no id can accidentally spell a word.
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Number of characters in a [`SessionId`]. Twelve base32 characters is
/// 60 bits: 40 bits of millisecond timestamp (a ~34-year range, which
/// outlives any plausible use of this tool) followed by 20 bits of
/// randomness.
///
/// **Length is a load-bearing design constraint, not cosmetic.** Two
/// separate Windows limits push on it from different directions:
///
/// 1. Worktree paths. Sessions get `<repo>/.sessionmgr-worktrees/<id>`
///    (Phase 2), appended to a target repo path this tool does not
///    control and cannot shorten.
/// 2. `AF_UNIX` socket paths, which are the harder limit of the two:
///    `sun_path` gives a hard **107 usable bytes for the entire path**,
///    and on Windows `std::env::temp_dir()` alone
///    (`C:\Users\<user>\AppData\Local\Temp`) can already spend 40-plus of
///    them. `rusty_prime_agent` hit exactly this and had to shorten its
///    own test state-root names to recover the budget.
///
/// Hence 12 characters rather than a full 26-character ULID: the same
/// time-ordered-prefix property, less than half the path cost.
const ID_LEN: usize = 12;

/// A session identifier: time-ordered, collision-resistant, and short
/// enough to be spent freely on path components.
///
/// Time-ordered because the high bits are a millisecond timestamp, so
/// lexicographic sort is creation order -- which makes `sessionmgr list`
/// stable and readable without carrying a separate sort key.
///
/// Constructed via [`SessionId::new`], which takes the clock reading and
/// random bits as arguments rather than sourcing them: this crate is
/// I/O-free by design, and passing them in is also what lets tests
/// construct a specific, reproducible id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIdError {
    /// Wrong length. Carried explicitly rather than folded into
    /// `BadCharacter` so a caller can tell "truncated/padded" apart from
    /// "corrupted", which are different bugs.
    WrongLength { got: usize },
    /// A character outside the Crockford alphabet.
    BadCharacter { got: char },
}

impl std::fmt::Display for SessionIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionIdError::WrongLength { got } => {
                write!(f, "session id must be {ID_LEN} characters, got {got}")
            }
            SessionIdError::BadCharacter { got } => {
                write!(f, "session id contains invalid character `{got}`")
            }
        }
    }
}

impl std::error::Error for SessionIdError {}

impl SessionId {
    /// Builds an id from a millisecond timestamp and 20 bits of
    /// randomness.
    ///
    /// Both inputs are arguments rather than being read here: see the
    /// crate docs for why this crate sources neither the clock nor
    /// entropy. Bits above each input's budget are discarded rather than
    /// rejected -- a timestamp beyond the 40-bit range or a `rand` with
    /// high bits set is not a caller error worth an `Err` at every call
    /// site, and truncation preserves the properties that matter
    /// (ordering within the range, randomness in the low bits).
    pub fn new(millis: u64, rand: u32) -> Self {
        let value = ((millis & 0xff_ffff_ffff) << 20) | u64::from(rand & 0x000f_ffff);
        let mut out = [0u8; ID_LEN];
        // Most-significant character first, so lexicographic order
        // matches numeric order and therefore creation order.
        for (i, slot) in out.iter_mut().enumerate() {
            let shift = 5 * (ID_LEN - 1 - i);
            *slot = ALPHABET[((value >> shift) & 0x1f) as usize];
        }
        // The bytes came from `ALPHABET`, which is ASCII, so this cannot
        // fail; `from_utf8_lossy` avoids an `expect` in non-test code
        // without needing a `Result` on an infallible path.
        SessionId(String::from_utf8_lossy(&out).into_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for SessionId {
    type Err = SessionIdError;

    /// Validates rather than trusting. Session ids arrive from the
    /// command line, from a socket, and from `state.json` on disk, and
    /// every one of those becomes a **path component** -- so an
    /// unvalidated id is a path-traversal primitive (`../../..`), not
    /// just a lookup miss. Restricting to a fixed length and a
    /// 32-character alphabet with no `.` or separator makes that
    /// structurally impossible rather than something a later caller has
    /// to remember to check.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != ID_LEN {
            return Err(SessionIdError::WrongLength { got: s.len() });
        }
        if let Some(bad) = s.chars().find(|c| !ALPHABET.contains(&(*c as u8))) {
            return Err(SessionIdError::BadCharacter { got: bad });
        }
        Ok(SessionId(s.to_owned()))
    }
}

impl TryFrom<String> for SessionId {
    type Error = SessionIdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<SessionId> for String {
    fn from(id: SessionId) -> String {
        id.0
    }
}

/// What kind of workspace a session runs against.
///
/// The three-way split is replicated deliberately rather than simplified
/// away. It would be tempting to always isolate -- it is simpler and
/// safer -- but the three distinct session-start actions are a real part
/// of the model being matched, and a same-directory session is a
/// legitimate choice a user makes knowingly, not an oversight to be
/// protected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionKind {
    /// A plain shell. No agent CLI, no git worktree, no repository.
    ///
    /// Built first, in the walking skeleton, because it carries zero
    /// agent-CLI uncertainty -- it is a shell, not an agent -- so it
    /// proved the daemon/worker/detach architecture without also
    /// depending on the unresolved "needs input" detection problem.
    PlainTerminal,
    /// Runs directly in the repository's own working copy.
    ///
    /// **Unisolated, and deliberately unprotected**: concurrent
    /// same-directory sessions share a working copy and an index and can
    /// collide. See [`crate::workspace::Workspace::same_directory`].
    SameDirectory,
    /// Isolated in its own git worktree on its own branch.
    ///
    /// The reason this project exists: it is the one capability nothing
    /// on the market combines with Windows support.
    Worktree,
}

impl SessionKind {
    /// Does this kind need a repository to be created against?
    pub fn needs_repo(self) -> bool {
        matches!(self, SessionKind::SameDirectory | SessionKind::Worktree)
    }
}

/// Which agent CLI a session runs, if any.
///
/// `None` on a session (a plain command or shell, or one created without
/// `--agent`) means exactly what it says: no adapter, no `needs_input`
/// detection beyond tier-2 process exit, which every session already
/// gets regardless. `Gemini`'s own adapter is built from its shipped
/// source and hooks reference docs rather than a live-captured session
/// (no credentials on this machine to run a real one) -- see
/// `sessionmgr_agents::gemini`'s own module docs for exactly what that
/// does and does not mean for this variant's confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Gemini,
}

/// Where a session is in its lifecycle.
///
/// PLAN.md states the machine as
/// `Created -> Running -> (NeedsInput | Errored) -> Merged | Discarded`.
/// `Merged`/`Discarded` are worktree outcomes and therefore Phase 2;
/// Phase 1's terminal state is [`SessionStatus::Closed`].
///
/// [`SessionStatus::Crashed`] is kept distinct from
/// [`SessionStatus::Errored`] rather than merged into it, because the two
/// mean genuinely different things to a user and to recovery: `Errored`
/// is "the thing you asked for ran and failed" (a non-zero exit -- the
/// tier-2 signal PLAN.md calls free and always reliable), while `Crashed`
/// is "the worker process died without reporting anything", which is a
/// failure of this tool, not of the user's command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    /// Record exists; no worker confirmed running yet.
    Created,
    /// A worker is running and the session is live.
    Running,
    /// The session is blocked waiting on the user.
    ///
    /// Phase 1 never produces this -- nothing detects it yet; that is the
    /// whole subject of the Phase 1 hook spike and the Phase 3 adapters.
    /// It is modelled now because the state machine's edges around it
    /// (notably close-while-needs-input) are exactly the cases PLAN.md's
    /// testing strategy calls out for unit coverage.
    NeedsInput,
    /// The session's process exited successfully.
    Finished,
    /// The session's process exited non-zero.
    Errored,
    /// The worker process died without reporting an outcome.
    Crashed,
    /// Torn down by the user.
    ///
    /// The terminal state for sessions with no branch of their own --
    /// `PlainTerminal` and `SameDirectory`. Worktree sessions end in
    /// [`Self::Merged`] or [`Self::Discarded`] instead, which say what
    /// happened to the work rather than merely that it stopped.
    Closed,
    /// Torn down, and the session's branch was merged back.
    Merged,
    /// Torn down, and the session's branch and worktree were thrown away.
    Discarded,
}

impl SessionStatus {
    /// Is this a state nothing further happens from?
    ///
    /// Note `Finished`/`Errored`/`Crashed` are **not** terminal: a session
    /// whose process has exited still holds a transcript and a worktree,
    /// so it remains closeable. Only the three torn-down states end the
    /// line.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SessionStatus::Closed | SessionStatus::Merged | SessionStatus::Discarded
        )
    }

    /// Has the underlying process finished, one way or another?
    pub fn is_exited(self) -> bool {
        matches!(
            self,
            SessionStatus::Finished | SessionStatus::Errored | SessionStatus::Crashed
        )
    }

    /// Should a worker be running for a session in this state?
    ///
    /// This is the question the supervisor asks of every `state.json` it
    /// finds on startup -- see [`crate::recovery::decide_recovery`].
    pub fn expects_live_worker(self) -> bool {
        matches!(
            self,
            SessionStatus::Created | SessionStatus::Running | SessionStatus::NeedsInput
        )
    }
}

/// A rejected state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub from: SessionStatus,
    pub to: SessionStatus,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid session transition from {:?} to {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for TransitionError {}

/// A pointer to the OS process backing a session, and the fingerprint
/// needed to tell it apart from an unrelated process that later inherits
/// the same pid.
///
/// The fingerprint is `Option` because not every platform can supply one;
/// see [`crate::recovery::decide_recovery`] for which way that ambiguity
/// is resolved and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRef {
    pub pid: u32,
    /// Opaque per-platform process start-time fingerprint. Only ever
    /// compared for equality, never parsed.
    pub start_fingerprint: Option<String>,
}

/// A session record: the domain object the daemon persists to
/// `state.json` and serves to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub kind: SessionKind,
    pub status: SessionStatus,
    /// The command the session runs. For `PlainTerminal` this is the
    /// user's shell.
    pub command: Vec<String>,
    /// The detached worker process supervising this session, once one has
    /// been spawned.
    pub worker: Option<WorkerRef>,
    /// The child the *worker* spawned -- the shell in Phase 1, an agent
    /// CLI later.
    ///
    /// Recorded separately and deliberately: PLAN.md's corrected close
    /// path terminates **both** the worker pid and this one, because
    /// killing only the worker leaves the child orphaned with no
    /// remediation path (adversarial finding #2). A pid this record does
    /// not carry is a pid nothing can clean up after a crash.
    pub child: Option<WorkerRef>,
    /// The repository, working directory, and branch this session runs
    /// against. `None` for a [`SessionKind::PlainTerminal`] session,
    /// which has no repository at all.
    ///
    /// `#[serde(default)]` so records written before worktree support
    /// existed still load: a `state.json` is read by whatever version of
    /// this tool is installed later, not necessarily the one that wrote
    /// it, and a session that survived an upgrade is exactly the kind
    /// this project promises not to lose.
    #[serde(default)]
    pub workspace: Option<Workspace>,
    /// Does this session's process run on a real terminal?
    ///
    /// Normally yes, and for an agent session it is not optional:
    /// interactive agent CLIs refuse to start without one (ADR-0002).
    ///
    /// `#[serde(default)]` gives `false`, which is deliberately the
    /// *truthful* answer for a record written before terminals existed
    /// here -- those sessions really were piped. Defaulting to `true`
    /// would relabel history.
    #[serde(default)]
    pub pty: bool,
    /// Millisecond timestamp, supplied by the caller (this crate does not
    /// read the clock).
    pub created_at_millis: u64,
    /// Set when the session's process exits.
    pub exit_code: Option<i32>,
    /// Which agent CLI this session runs, if any. `None` for a session
    /// created without `--agent`.
    ///
    /// `#[serde(default)]` for the same reason `workspace` and `pty`
    /// are: a record written before this field existed must still load.
    #[serde(default)]
    pub agent: Option<AgentKind>,
    /// A user-chosen display label, purely cosmetic -- distinct from the
    /// worktree branch name, which stays `sessionmgr/<id>` no matter what
    /// this is set to. `None` until renamed (the TUI command palette's
    /// `rename` action; CAPABILITIES.md's Xirp-observed "renaming a
    /// session").
    ///
    /// `#[serde(default)]` for the same reason as the fields above: a
    /// record written before this field existed must still load.
    #[serde(default)]
    pub name: Option<String>,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: SessionId,
        kind: SessionKind,
        command: Vec<String>,
        workspace: Option<Workspace>,
        pty: bool,
        created_at_millis: u64,
        agent: Option<AgentKind>,
    ) -> Self {
        Session {
            id,
            kind,
            status: SessionStatus::Created,
            command,
            worker: None,
            child: None,
            workspace,
            pty,
            created_at_millis,
            exit_code: None,
            agent,
            name: None,
        }
    }

    /// Sets (or, given `None`, clears) this session's display label.
    /// Purely cosmetic bookkeeping, not a state-machine transition --
    /// unlike `transition_to`/`record_exit` this cannot fail.
    pub fn rename(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// The teardown status appropriate to how this session was closed.
    ///
    /// `Merged`/`Discarded` record what happened to the *work*, so they
    /// are only ever reached when something actually happened to it:
    ///
    /// - **No disposition** is `Closed` — the processes were stopped and
    ///   the worktree and branch were left exactly where they were.
    ///   Reporting that as `Discarded` would tell the user their work was
    ///   thrown away when it is still sitting on disk, which is a lie in
    ///   the more alarming direction.
    /// - **A session owning no branch** is always `Closed`, whatever was
    ///   asked for. `--discard` on a same-directory session cannot mean
    ///   "delete the user's repository", and there is nothing else it
    ///   could refer to.
    pub fn teardown_status(&self, disposition: Option<Disposition>) -> SessionStatus {
        let owns_branch = self
            .workspace
            .as_ref()
            .map(|w| w.branch.is_some())
            .unwrap_or(false);
        match disposition {
            None => SessionStatus::Closed,
            _ if !owns_branch => SessionStatus::Closed,
            Some(Disposition::Merge) => SessionStatus::Merged,
            Some(Disposition::Discard) => SessionStatus::Discarded,
        }
    }

    /// Is `to` a legal next status from the current one?
    ///
    /// Encoded as one table rather than scattered across the call sites
    /// that mutate status, so "which transitions exist" is a single
    /// readable thing and the unit tests can enumerate it exhaustively.
    pub fn can_transition_to(&self, to: SessionStatus) -> bool {
        use SessionStatus::*;
        match (self.status, to) {
            // Nothing leaves a torn-down session. This is what makes
            // double-close an error rather than a silent no-op.
            (from, _) if from.is_terminal() => false,

            // Any live session can be torn down, by any of the three
            // teardown outcomes.
            (_, Closed | Merged | Discarded) => true,

            // A session can crash from any state where a worker was
            // supposed to be running -- that is precisely the case the
            // supervisor detects on restart.
            (from, Crashed) => from.expects_live_worker(),

            (Created, Running) => true,
            // Created -> Finished/Errored covers a command that exits
            // before the worker ever reports itself running: a bad
            // executable path is the ordinary way this happens, and
            // forcing it through `Running` first would be a lie.
            (Created, Finished | Errored) => true,

            (Running, NeedsInput | Finished | Errored) => true,
            (NeedsInput, Running | Finished | Errored) => true,

            // An exited session does not resume. Phase 6+'s
            // fork/switch-agent work is a *new* session seeded from an
            // old one, never a resurrection of this record -- see
            // PLAN.md, which gates both on an unproven per-CLI
            // state-translation primitive.
            (Finished | Errored | Crashed, _) => false,

            // Everything else is rejected, which notably includes
            // re-entering the state you are already in (callers wanting
            // idempotence check first) and any move back to `Created`,
            // which is a creation-time-only state.
            _ => false,
        }
    }

    /// Moves to `to`, or returns [`TransitionError`] leaving the session
    /// untouched.
    pub fn transition_to(&mut self, to: SessionStatus) -> Result<(), TransitionError> {
        if !self.can_transition_to(to) {
            return Err(TransitionError {
                from: self.status,
                to,
            });
        }
        self.status = to;
        Ok(())
    }

    /// Records a process exit, moving to `Finished` or `Errored`
    /// according to the exit code.
    ///
    /// This is PLAN.md's **tier-2 signal**: process exit status is the
    /// one status source that is free, always reliable, available for all
    /// three CLIs, and independent of both hook support and output
    /// pattern-matching. Everything uncertain about status detection
    /// lives in the other two tiers; this one cannot be wrong.
    pub fn record_exit(&mut self, code: Option<i32>) -> Result<(), TransitionError> {
        let to = if code == Some(0) {
            SessionStatus::Finished
        } else {
            SessionStatus::Errored
        };
        self.transition_to(to)?;
        self.exit_code = code;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new(
            SessionId::new(1_700_000_000_000, 42),
            SessionKind::PlainTerminal,
            vec!["sh".to_owned()],
            None,
            true,
            1_700_000_000_000,
            None,
        )
    }

    #[test]
    fn ids_are_fixed_length_and_round_trip() {
        let id = SessionId::new(1_700_000_000_000, 42);
        assert_eq!(id.as_str().len(), ID_LEN);
        let parsed: SessionId = id.as_str().parse().expect("own id must re-parse");
        assert_eq!(parsed, id);
    }

    #[test]
    fn ids_sort_in_creation_order() {
        // The property `sessionmgr list` depends on for stable ordering
        // without a separate sort key.
        let earlier = SessionId::new(1_700_000_000_000, 999);
        let later = SessionId::new(1_700_000_000_001, 0);
        assert!(
            earlier < later,
            "a later timestamp must sort after an earlier one regardless of the random suffix"
        );
    }

    #[test]
    fn ids_with_the_same_millisecond_differ_by_random_bits() {
        let a = SessionId::new(1_700_000_000_000, 1);
        let b = SessionId::new(1_700_000_000_000, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn parsing_rejects_path_traversal_and_junk() {
        // The reason `FromStr` validates at all: every id becomes a path
        // component, so these are traversal attempts, not typos.
        for bad in ["../../etc", "..", "a/b", "", "short", "toolongtobevalid"] {
            assert!(
                bad.parse::<SessionId>().is_err(),
                "`{bad}` must not parse as a session id"
            );
        }
        // Right length, wrong alphabet: `i`/`l`/`o`/`u` are excluded.
        assert_eq!(
            "iiiiiiiiiiii".parse::<SessionId>(),
            Err(SessionIdError::BadCharacter { got: 'i' })
        );
        assert_eq!(
            "abc".parse::<SessionId>(),
            Err(SessionIdError::WrongLength { got: 3 })
        );
    }

    #[test]
    fn a_new_session_starts_created_with_no_processes() {
        let s = session();
        assert_eq!(s.status, SessionStatus::Created);
        assert!(s.worker.is_none());
        assert!(s.child.is_none());
        assert!(s.exit_code.is_none());
    }

    #[test]
    fn the_happy_path_runs_and_closes() {
        let mut s = session();
        assert!(s.transition_to(SessionStatus::Running).is_ok());
        assert!(s.transition_to(SessionStatus::Closed).is_ok());
        assert!(s.status.is_terminal());
    }

    #[test]
    fn double_close_is_rejected_not_silently_ignored() {
        let mut s = session();
        assert!(s.transition_to(SessionStatus::Running).is_ok());
        assert!(s.transition_to(SessionStatus::Closed).is_ok());
        assert_eq!(
            s.transition_to(SessionStatus::Closed),
            Err(TransitionError {
                from: SessionStatus::Closed,
                to: SessionStatus::Closed
            })
        );
        // And the rejected transition left the session untouched.
        assert_eq!(s.status, SessionStatus::Closed);
    }

    #[test]
    fn closing_while_needs_input_is_allowed() {
        // Called out by PLAN.md's testing strategy specifically: a
        // session blocked on the user is the most likely one to be
        // abandoned, so it must be closeable.
        let mut s = session();
        assert!(s.transition_to(SessionStatus::Running).is_ok());
        assert!(s.transition_to(SessionStatus::NeedsInput).is_ok());
        assert!(s.transition_to(SessionStatus::Closed).is_ok());
    }

    #[test]
    fn needs_input_can_resume_to_running() {
        let mut s = session();
        assert!(s.transition_to(SessionStatus::Running).is_ok());
        assert!(s.transition_to(SessionStatus::NeedsInput).is_ok());
        assert!(s.transition_to(SessionStatus::Running).is_ok());
    }

    #[test]
    fn an_exited_session_never_resumes() {
        for terminal in [
            SessionStatus::Finished,
            SessionStatus::Errored,
            SessionStatus::Crashed,
        ] {
            let mut s = session();
            s.status = terminal;
            for to in [
                SessionStatus::Running,
                SessionStatus::NeedsInput,
                SessionStatus::Created,
            ] {
                assert!(
                    s.transition_to(to).is_err(),
                    "{terminal:?} must not resume to {to:?}"
                );
            }
            // ...but is still closeable, because it still owns a
            // transcript (and, from Phase 2, a worktree) to clean up.
            assert!(s.transition_to(SessionStatus::Closed).is_ok());
        }
    }

    #[test]
    fn a_session_can_crash_only_from_a_state_that_expected_a_worker() {
        for from in [
            SessionStatus::Created,
            SessionStatus::Running,
            SessionStatus::NeedsInput,
        ] {
            let mut s = session();
            s.status = from;
            assert!(
                s.transition_to(SessionStatus::Crashed).is_ok(),
                "{from:?} expects a live worker, so it must be able to crash"
            );
        }
        for from in [
            SessionStatus::Finished,
            SessionStatus::Errored,
            SessionStatus::Closed,
        ] {
            let mut s = session();
            s.status = from;
            assert!(
                s.transition_to(SessionStatus::Crashed).is_err(),
                "{from:?} has no live worker, so it cannot crash"
            );
        }
    }

    #[test]
    fn recording_a_zero_exit_finishes_and_nonzero_errors() {
        let mut s = session();
        assert!(s.transition_to(SessionStatus::Running).is_ok());
        assert!(s.record_exit(Some(0)).is_ok());
        assert_eq!(s.status, SessionStatus::Finished);
        assert_eq!(s.exit_code, Some(0));

        let mut s = session();
        assert!(s.transition_to(SessionStatus::Running).is_ok());
        assert!(s.record_exit(Some(3)).is_ok());
        assert_eq!(s.status, SessionStatus::Errored);
        assert_eq!(s.exit_code, Some(3));
    }

    #[test]
    fn an_exit_with_no_code_is_an_error_not_a_success() {
        // A process killed by a signal reports no exit code. Treating
        // that as success would report a killed agent as having finished
        // its work.
        let mut s = session();
        assert!(s.transition_to(SessionStatus::Running).is_ok());
        assert!(s.record_exit(None).is_ok());
        assert_eq!(s.status, SessionStatus::Errored);
    }

    #[test]
    fn a_command_that_dies_immediately_exits_without_passing_through_running() {
        // A bad executable path is the ordinary way this happens.
        let mut s = session();
        assert!(s.record_exit(Some(127)).is_ok());
        assert_eq!(s.status, SessionStatus::Errored);
    }

    fn worktree_session() -> Session {
        let id = SessionId::new(1_700_000_000_000, 42);
        Session::new(
            id.clone(),
            SessionKind::Worktree,
            vec!["sh".to_owned()],
            Some(Workspace::worktree(std::path::PathBuf::from("/repo"), &id)),
            true,
            1_700_000_000_000,
            None,
        )
    }

    #[test]
    fn a_close_with_no_disposition_never_reports_work_as_discarded() {
        // The bug this guards was real and was caught by the worktree
        // tests: a bare `close` left the worktree on disk (correctly) but
        // recorded the session as `Discarded`, telling the user their
        // work had been thrown away when it had not.
        assert_eq!(
            worktree_session().teardown_status(None),
            SessionStatus::Closed
        );
    }

    #[test]
    fn dispositions_map_to_outcome_statuses_for_a_session_that_owns_a_branch() {
        assert_eq!(
            worktree_session().teardown_status(Some(Disposition::Merge)),
            SessionStatus::Merged
        );
        assert_eq!(
            worktree_session().teardown_status(Some(Disposition::Discard)),
            SessionStatus::Discarded
        );
    }

    #[test]
    fn a_session_owning_no_branch_always_closes_whatever_was_asked_for() {
        // `--discard` on a same-directory session cannot mean "delete the
        // user's repository"; there is no branch of ours to discard.
        let mut s = session();
        s.kind = SessionKind::SameDirectory;
        s.workspace = Some(Workspace::same_directory(std::path::PathBuf::from("/repo")));
        for disposition in [None, Some(Disposition::Merge), Some(Disposition::Discard)] {
            assert_eq!(s.teardown_status(disposition), SessionStatus::Closed);
        }
        // And a plain terminal has no workspace at all.
        assert_eq!(
            session().teardown_status(Some(Disposition::Discard)),
            SessionStatus::Closed
        );
    }

    #[test]
    fn the_outcome_statuses_are_terminal_just_like_closed() {
        for terminal in [
            SessionStatus::Closed,
            SessionStatus::Merged,
            SessionStatus::Discarded,
        ] {
            let mut s = worktree_session();
            s.status = terminal;
            assert!(terminal.is_terminal());
            assert!(
                s.transition_to(SessionStatus::Closed).is_err(),
                "{terminal:?} must not be closeable again"
            );
        }
    }

    #[test]
    fn a_rejected_exit_leaves_the_exit_code_unrecorded() {
        // `record_exit` must not half-apply: if the transition is
        // rejected, the code must not be written either.
        let mut s = session();
        s.status = SessionStatus::Closed;
        assert!(s.record_exit(Some(1)).is_err());
        assert_eq!(s.exit_code, None);
    }
}
