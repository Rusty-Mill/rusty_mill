//! Process types and spawning trait (RFC v2 §5.4, decision D-7).
//!
//! What is here now is the surface the reference consumer (`coreutils`)
//! and the parity suite need: the spawn specification types, a uniformly
//! *decoded* [`ExitStatus`], and the object-safe [`Spawner`]/[`Child`]
//! pair with consuming `wait` (double-wait is unrepresentable by
//! construction — the v1 scaffold's use-after-close class cannot be
//! written against this API).
//!
//! What is deliberately NOT here yet (RFC v2 §5.6): the reactor
//! (wait-any), process groups, pipes, and pty. Those shapes are contracted
//! to arrive from rush at the R2 hoist with their semantics already proven;
//! designing them speculatively here is exactly what the consumer gate
//! (§3) forbids.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

use crate::error::Result;

/// How the child's environment is constructed.
#[derive(Debug, Clone, Default)]
pub enum EnvSpec {
    /// Inherit the parent environment unchanged.
    #[default]
    Inherit,
    /// Start empty; only the given variables are set.
    Explicit(BTreeMap<OsString, OsString>),
}

/// Stdio wiring for the child.
///
/// Not `Copy`/`Clone`/`PartialEq`/`Eq`, unlike most small value types in
/// this module: [`Stdio::File`] owns a `Box<dyn` [`crate::fs::File`]`>`,
/// an open OS handle with no honest "clone for a log/comparison" meaning
/// (duplicating it is [`crate::fs::File::try_clone`], a real OS call,
/// not a value-type copy). Consumers that need to distinguish variants
/// use `matches!` rather than `==`.
#[derive(Default)]
pub enum Stdio {
    #[default]
    Inherit,
    Null,
    /// Wire this slot to a pipe whose parent end is retrieved from the
    /// spawned child via [`Child::take_stdin`]/[`Child::take_stdout`]/
    /// [`Child::take_stderr`] — the write end for stdin, read ends for
    /// stdout/stderr. Deadlock contract in `docs/behavior/process.md`:
    /// drain (or drop) captured output before blocking in `wait`, and
    /// drop the stdin end to deliver EOF.
    Pipe,
    /// Wire this slot to an already-open [`crate::fs::File`] — D5
    /// (`docs/extraction-map.md`), forced by shell redirects (`>
    /// file`/`>> file`/`< file`/`2>&1`/`&> file`, rustils#51): the
    /// caller opens (or [`crate::fs::File::try_clone`]s, for the
    /// `2>&1`/`&> file` shared-description shape) the target file and
    /// hands ownership to this slot. Mechanism only — a backend wires
    /// the given file's underlying fd/handle onto the child's
    /// stdin/stdout/stderr (Unix: `dup2`-equivalent; Windows: an
    /// inheritable `DuplicateHandle`) without consuming or closing it,
    /// so the same [`crate::fs::File`] a caller already holds keeps
    /// working after `spawn` returns. `Spawner::spawn` fails
    /// `Unsupported` for a `File` value it didn't create itself (a
    /// foreign backend's `File` impl) rather than guessing how to
    /// extract a raw handle from it.
    File(Box<dyn crate::fs::File>),
}

impl std::fmt::Debug for Stdio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stdio::Inherit => write!(f, "Inherit"),
            Stdio::Null => write!(f, "Null"),
            Stdio::Pipe => write!(f, "Pipe"),
            // The boxed `dyn File` has no `Debug` bound of its own (RFC
            // v2 §5.1: object-safe traits here stay minimal) — printed
            // as a marker, not its contents.
            Stdio::File(_) => write!(f, "File(..)"),
        }
    }
}

/// Process-group placement for the child (RFC v2 §5.4 — groups are
/// first-class). Maps to `setpgid`-at-spawn on unix and a kill-on-close
/// Job Object joined before the first instruction on Windows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GroupSpec {
    /// Stay in the parent's group / no job object.
    #[default]
    Inherit,
    /// Lead a fresh group: the child (and everything it spawns) becomes
    /// [`Child::kill_tree`]'s target. On Windows the job is
    /// kill-on-close, so dropping the child without waiting terminates
    /// the tree (see `docs/behavior/process.md`).
    NewGroup,
    /// Join the group led by `pgid` — a pipeline stage 2..n placed into
    /// the group its first stage's spawn already created with
    /// `NewGroup` (D1's shape: the leader's own pid becomes the
    /// pipeline's pgid; every later stage joins it instead of leading
    /// its own). Race-free the same way `NewGroup` is: placed before the
    /// child's first instruction, not `setpgid` after the fact. Unix
    /// only — Windows has no numeric process-group id a spawn can join;
    /// `Spawner::spawn` fails `Unsupported` (divergence 008).
    JoinGroup(u32),
}

/// A fully-specified spawn request.
///
/// Built with [`Command`], executed by a [`Spawner`]. `argv` is a list of
/// discrete arguments end to end; any joining or quoting an OS requires is
/// backend-internal and never caller-visible (the Windows backend's quoting
/// module is the security boundary here — RFC v2 §5.4).
///
/// Not `Clone`, unlike most builder types: a [`Stdio::File`] slot owns an
/// open OS handle with no honest "clone for a log/re-spawn" meaning (see
/// [`Stdio`]'s own doc comment). A consumer that wants to log spawn
/// requests for test assertions (e.g. `platform-mock`'s `SpawnRecord`)
/// snapshots the fields it needs rather than cloning the whole value.
#[derive(Debug)]
pub struct Command {
    pub program: OsString,
    pub argv: Vec<OsString>,
    /// Working directory for the child. Always explicit — there is no
    /// "inherit ambient cwd" variant, by design: consumers own their cwd
    /// policy (rush virtualizes it; RFC v2 §5.3 rationale).
    pub cwd: OsString,
    pub env: EnvSpec,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
    pub group: GroupSpec,
    /// Set by [`Command::detach`]. See that method's doc comment for the
    /// full contract, including the `GroupSpec` combinations it refuses.
    pub detached: bool,
}

impl Command {
    pub fn new(program: impl Into<OsString>, cwd: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            argv: Vec::new(),
            cwd: cwd.into(),
            env: EnvSpec::default(),
            stdin: Stdio::default(),
            stdout: Stdio::default(),
            stderr: Stdio::default(),
            group: GroupSpec::default(),
            detached: false,
        }
    }

    #[must_use]
    pub fn arg(mut self, a: impl Into<OsString>) -> Self {
        self.argv.push(a.into());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.argv.extend(args.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn env(mut self, env: EnvSpec) -> Self {
        self.env = env;
        self
    }

    #[must_use]
    pub fn group(mut self, group: GroupSpec) -> Self {
        self.group = group;
        self
    }

    /// Detach the child from this process's session/console: it survives
    /// this process exiting (including crashing) and its terminal
    /// closing. Unix: `POSIX_SPAWN_SETSID` — the child becomes a new
    /// session **and** process-group leader (`pid == sid == pgid`) before
    /// its first instruction runs. Windows: `CREATE_NEW_PROCESS_GROUP |
    /// DETACHED_PROCESS` — no console, not a member of this process's
    /// Ctrl-C group.
    ///
    /// Composes **only** with [`GroupSpec::Inherit`] (the default) —
    /// every other `GroupSpec` is refused at spawn time, for two
    /// different reasons on the two backends, both real OS limitations
    /// rather than a shared implementation choice:
    /// - [`GroupSpec::JoinGroup`] is refused (`InvalidInput`) on
    ///   **every** backend: a fresh session cannot also join an
    ///   existing, different pgid — self-contradictory, not an OS
    ///   limitation.
    /// - [`GroupSpec::NewGroup`] is refused (`Unsupported`) on **every**
    ///   backend too, for two unrelated real reasons. Linux: `setsid`
    ///   always makes the child a session leader, and `setpgid(2)`
    ///   forbids changing a session leader's process group ID — even a
    ///   self-targeting `setpgid(0, 0)` no-op — so the underlying
    ///   `posix_spawn` call would fail `EPERM` outright (confirmed
    ///   against a real kernel, not a documentation reading). Windows: a
    ///   kill-on-close Job Object is torn down — killing every member —
    ///   the instant every handle to it closes, which the OS does
    ///   unconditionally when this process terminates for any reason,
    ///   including a crash; combined with `detach`, that would silently
    ///   defeat the very guarantee `detach` promises.
    ///
    /// A caller that wants a detached child it can still `kill_tree`
    /// later uses the existing two-step path on either backend:
    /// `detach()` alone at spawn — which, on Linux, already gives the
    /// child its own pgid via `setsid`, so `kill_tree` has a sound
    /// target with no `NewGroup` needed at all — then, on Windows,
    /// [`Spawner::adopt`] on its pid when it actually decides to kill
    /// it (Linux's `Spawner::adopt` stays `Unsupported` regardless,
    /// divergence 010 — irrelevant here since Linux never needed it for
    /// this case).
    #[must_use]
    pub fn detach(mut self) -> Self {
        self.detached = true;
        self
    }
}

/// How a child terminated — decoded uniformly on every backend.
///
/// Linux backends decode the raw `waitpid` status word (`WIFEXITED` /
/// `WIFSIGNALED`); Windows backends report the exit code. A raw status
/// word must never cross this boundary (pins v1 scaffold bug B-5; the
/// parity suite's permanent sentinel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// Normal exit with the given code.
    Code(i32),
    /// Terminated by a signal (unix); never produced on Windows.
    Signaled(i32),
    /// Stopped by a signal (`WIFSTOPPED`/`SIGTSTP`-class — Ctrl-Z), not
    /// terminated: the process is still alive and waitable again later.
    /// Only produced by [`Child::wait_job`]/[`Child::try_wait_job`]
    /// (D10) — the plain `wait`/`try_wait` pair never requests
    /// `WUNTRACED` and so can never observe it. Unix only; never
    /// produced on Windows (no job-control stop analog — D8).
    Stopped(i32),
    /// Resumed from a stop (`WIFCONTINUED`/`SIGCONT`) — like `Stopped`,
    /// not terminal: the process is running again. Same
    /// `wait_job`/`try_wait_job`-only, Unix-only scoping as `Stopped`.
    Continued,
}

impl ExitStatus {
    pub fn success(self) -> bool {
        matches!(self, ExitStatus::Code(0))
    }
}

/// Portable signal identities for [`Child::kill_tree`]/[`Child::kill_single`]
/// (extraction map D1's `kill`/`fg`/`bg` builtins: `kill_cmd`'s
/// `-SIG`/`-9`/`-CONT` argument, `fg_cmd`/`bg_cmd`'s resume step) — a small
/// mechanism-level set, the same naming-not-raw-numbers discipline
/// [`crate::events::SignalEvent`] already applies to received signals.
/// `Kill` is the only member guaranteed on every backend: Windows has no
/// general signal-delivery mechanism, so every other variant is
/// `Unsupported` there (divergence 008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Graceful termination request: `SIGTERM`.
    Term,
    /// Interrupt: `SIGINT`.
    Int,
    /// Controlling-terminal / session hangup: `SIGHUP`.
    Hup,
    /// Quit with core-dump semantics: `SIGQUIT`.
    Quit,
    /// Unconditional termination: `SIGKILL`. Windows maps this to
    /// `TerminateJobObject`/`TerminateProcess` (unchanged from this
    /// trait's pre-`Signal` behavior; divergence 001 still applies to
    /// the resulting status).
    Kill,
    /// Suspend: `SIGSTOP` — the Ctrl-Z half of job control.
    Stop,
    /// Resume a stopped process: `SIGCONT` — `fg`/`bg`'s resume step.
    Cont,
}

/// A spawned child. Object-safe.
pub trait Child {
    /// Wait for termination, consuming the child.
    ///
    /// Consuming `self` makes double-wait — and therefore the
    /// wait-after-close bug class — unrepresentable.
    fn wait(self: Box<Self>) -> Result<ExitStatus>;

    /// OS process identifier, for display/diagnostics.
    fn id(&self) -> u32;

    /// Deliver `sig` to the child's whole group (the child and every
    /// descendant). Requires [`GroupSpec::NewGroup`] or
    /// [`GroupSpec::JoinGroup`] at spawn — on a child spawned with
    /// `Inherit` this fails `Unsupported` rather than guessing at a
    /// target (killing the parent's own group is the alternative, and it
    /// is never what the caller meant). `Signal::Kill` must still be
    /// `wait`ed to observe the resulting status; the form that status
    /// takes is OS-divergent (divergence 001). Every `Signal` other than
    /// `Kill` is `Unsupported` on Windows (divergence 008): there is no
    /// OS mechanism to deliver an arbitrary signal identity to a process
    /// this backend didn't just terminate.
    fn kill_tree(&self, sig: Signal) -> Result<()>;

    /// Deliver `sig` to the child process only — descendants survive.
    /// Same `Signal::Kill`-only guarantee on Windows as [`kill_tree`](
    /// Child::kill_tree) (divergence 008).
    fn kill_single(&self, sig: Signal) -> Result<()>;

    /// Non-blocking poll: `Some(status)` if the child has terminated,
    /// `None` if it is still running. A child that has reported a status
    /// here keeps reporting the same status (backends stash the reaped
    /// result), and the consuming [`Child::wait`] afterwards returns it —
    /// polling never loses the exit status.
    fn try_wait(&mut self) -> Result<Option<ExitStatus>>;

    /// Blocking counterpart of [`Child::try_wait_job`]: block until the
    /// child terminates, stops, or (if already stopped) continues — the
    /// `WUNTRACED`/`WCONTINUED` half of wait (D10), the foreground-job
    /// shape `rush`'s `wait_pgid` needs to notice a Ctrl-Z'd pipeline
    /// without conflating it with exit/signal termination. Does not
    /// consume `self`: unlike plain [`Child::wait`], a `Stopped`/
    /// `Continued` result is not terminal, so the caller keeps the
    /// child and may call again. A terminal `Code`/`Signaled` result IS
    /// stashed exactly like `try_wait` does, so a later `wait()` returns
    /// it directly rather than re-blocking. Unix only — `Unsupported` on
    /// Windows (no stop/continue analog, D8).
    fn wait_job(&mut self) -> Result<ExitStatus>;

    /// Non-blocking counterpart of [`Child::wait_job`]:
    /// `WNOHANG|WUNTRACED|WCONTINUED`, the shape `rush`'s
    /// `reap_background` needs to track a background job transitioning
    /// stopped ↔ running. `None` while the child is running and has
    /// neither stopped nor continued since the last poll. Same
    /// stash-on-terminal, non-consuming, Unix-only contract as
    /// [`Child::wait_job`].
    fn try_wait_job(&mut self) -> Result<Option<ExitStatus>>;

    /// The parent's write end of the child's stdin, if that slot was
    /// [`Stdio::Pipe`]. Yields `Some` exactly once; dropping the handle
    /// delivers EOF to the child.
    fn take_stdin(&mut self) -> Option<Box<dyn crate::fs::File>>;

    /// The parent's read end of the child's stdout, if piped. `Some`
    /// exactly once; reads return 0 at EOF (child closed its end).
    fn take_stdout(&mut self) -> Option<Box<dyn crate::fs::File>>;

    /// The parent's read end of the child's stderr, if piped. `Some`
    /// exactly once.
    fn take_stderr(&mut self) -> Option<Box<dyn crate::fs::File>>;

    /// Downcast hook so a backend's [`Spawner::wait_any`] can reach its
    /// own children's OS handles through the object-safe trait. Not for
    /// consumers.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// A kill-scoped handle over a process a [`Spawner`] did not itself spawn —
/// e.g. one created by a third-party library ([`Spawner::adopt`], rustils#47:
/// `portable-pty::Child::process_id()` is the forcing case, for a PTY
/// session `nexus-terminal` spawns through `portable-pty` rather than
/// through this crate). Deliberately narrower than [`Child`]: an adopted
/// pid was never spawned through this crate, so `wait`/stdio have no
/// meaning here — the caller already owns whatever wait/stdio mechanism
/// the third-party spawn gave them. Object-safe.
pub trait GroupHandle {
    /// Same contract as [`Child::kill_tree`], targeting the adopted pid's
    /// own tree.
    fn kill_tree(&self, sig: Signal) -> Result<()>;

    /// Same contract as [`Child::kill_single`], targeting the adopted pid
    /// itself.
    fn kill_single(&self, sig: Signal) -> Result<()>;
}

/// Block until *some* child in `children` terminates, for up to `timeout`
/// (`None` = wait forever). Returns `Some(index)` of a terminated child —
/// retrieve the status via that child's [`Child::try_wait`]/[`Child::wait`]
/// — or `None` on timeout. An empty slice is `InvalidInput` (mirrors the
/// OS primitives' own refusal).
///
/// This is the wait-any *seed* (extraction map step 3): a portable
/// poll-over-`try_wait` loop with a 10ms tick — the same coarser stand-in
/// rush ran before adopting `WaitForMultipleObjects`, correct on every
/// backend including the mock. The OS-multiplexed reactor (pidfd+poll on
/// Linux; `WaitForMultipleObjects` with the 64-handle limit absorbed
/// internally, RFC v2 §5.6) replaces this loop's internals at R3 without
/// changing this contract.
pub fn wait_any(
    children: &mut [Box<dyn Child>],
    timeout: Option<std::time::Duration>,
) -> Result<Option<usize>> {
    use crate::error::{ErrorKind, OsCode, PlatformError};
    if children.is_empty() {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "wait_any",
        ));
    }
    let start = std::time::Instant::now();
    const TICK: std::time::Duration = std::time::Duration::from_millis(10);
    loop {
        for (i, child) in children.iter_mut().enumerate() {
            if child.try_wait()?.is_some() {
                return Ok(Some(i));
            }
        }
        let elapsed = start.elapsed();
        let sleep = match timeout {
            Some(limit) if elapsed >= limit => return Ok(None),
            Some(limit) => TICK.min(limit - elapsed),
            None => TICK,
        };
        std::thread::sleep(sleep);
    }
}

/// A backend capable of spawning processes. Object-safe.
pub trait Spawner {
    fn spawn(&self, cmd: &Command) -> Result<Box<dyn Child>>;

    /// Resolve `program` against this backend's executable-lookup rules
    /// (PATH + exec bit on unix; PATH + PATHEXT on Windows). Mechanism
    /// only — policy layers (e.g. a shell's builtin/function precedence)
    /// live in the consumer.
    fn resolve(&self, program: &OsStr) -> Result<OsString>;

    /// Backend-multiplexed wait-any: same contract as the free
    /// [`wait_any`], which is also the default implementation. Native
    /// backends override with a real OS multiplexer (pidfd+`poll` on
    /// Linux; `WaitForMultipleObjects` on Windows with its 64-handle
    /// limit absorbed internally, RFC v2 §5.6) and fall back to the
    /// portable loop for children they don't recognize.
    fn wait_any(
        &self,
        children: &mut [Box<dyn Child>],
        timeout: Option<std::time::Duration>,
    ) -> Result<Option<usize>> {
        wait_any(children, timeout)
    }

    /// Adopt an already-running process this `Spawner` did not itself
    /// spawn into a fresh kill-on-close group, giving it
    /// [`GroupHandle::kill_tree`]/[`GroupHandle::kill_single`] semantics
    /// after the fact (rustils#47). Windows: `OpenProcess` + a fresh
    /// kill-on-close Job Object (`AssignProcessToJobObject`) — the same
    /// mechanism [`GroupSpec::NewGroup`] uses at spawn time, applied after
    /// the fact to a pid this backend didn't create. Unix: `Unsupported` —
    /// POSIX's `setpgid(pid, pgid)` can only retarget a process that is
    /// both the caller's own child *and* has not yet exec'd; by the time a
    /// caller has a pid to adopt (e.g. from
    /// `portable-pty::Child::process_id()`), the target has already
    /// exec'd, so there is no sound way to honor this on Unix — divergence
    /// **010**. Never attempted speculatively: a `setpgid` that
    /// sometimes works depending on timing would be worse than an honest
    /// refusal.
    fn adopt(&self, pid: u32) -> Result<Box<dyn GroupHandle>>;

    /// Is `pid` currently alive? Not tied to any [`Child`] this backend
    /// spawned — works for any pid, including one this process has no
    /// wait/kill relationship with (a `detach`ed worker's pid recorded
    /// earlier, or a third-party pid). Unix: `kill(pid, 0)` —
    /// `Ok(true)` on success *or* `EPERM` (the pid exists, just isn't
    /// signalable by this process), `Ok(false)` on `ESRCH` (no such
    /// process). Windows: `OpenProcess` + `GetExitCodeProcess`;
    /// `Ok(true)` if the code is `STILL_ACTIVE` or the open itself
    /// failed with anything other than "no such process"; `Ok(false)`
    /// otherwise. A zombie (Linux, exited but not yet reaped by its real
    /// parent) reports `true` here — `kill(pid, 0)` cannot distinguish
    /// "running" from "exited but unreaped"; see [`Spawner::is_zombie`]
    /// for that distinction.
    fn is_alive(&self, pid: u32) -> Result<bool>;

    /// Is `pid` a zombie (exited, not yet reaped by its real parent)?
    /// Linux: `/proc/<pid>/stat`'s state field is `Z`; a missing `/proc`
    /// entry means the pid doesn't exist at all, reported as `Ok(false)`
    /// — not a zombie, gone — not an error. Windows: always `Unsupported`
    /// (`docs/divergences.md` #015) — a Windows process handle stays
    /// valid and its exit code re-readable indefinitely after exit (see
    /// [`Child::try_wait`]'s own doc comment), so there is no distinct
    /// "exited but unreaped" state to observe; the question this method
    /// asks has no honest Windows answer.
    fn is_zombie(&self, pid: u32) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_builder_composes() {
        let c = Command::new("prog", "/work").arg("a").args(["b", "c"]);
        assert_eq!(c.argv.len(), 3);
        assert_eq!(c.cwd, OsString::from("/work"));
    }

    #[test]
    fn exit_status_is_decoded_semantics() {
        assert!(ExitStatus::Code(0).success());
        assert!(!ExitStatus::Code(1).success());
        assert!(!ExitStatus::Signaled(9).success());
    }
}
