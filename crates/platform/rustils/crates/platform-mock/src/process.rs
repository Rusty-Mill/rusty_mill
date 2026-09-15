//! Scripted process backend: spawn requests are matched against
//! pre-registered scripts and produce deterministic results. This is how
//! consumer process-orchestration logic is tested with zero real processes.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::sync::{Arc, Mutex};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{
    Child, Command, EnvSpec, ExitStatus, GroupHandle, GroupSpec, Signal, Spawner, Stdio,
};

/// A scripted response for a program name.
#[derive(Debug, Clone)]
pub struct Script {
    pub status: ExitStatus,
    /// Bytes served through `take_stdout` when the spawn piped stdout.
    pub stdout: Vec<u8>,
}

/// Which kind of [`Stdio`] wiring a slot requested — [`SpawnRecord`]'s
/// field type. Not the `Stdio` value itself: a [`Stdio::File`] owns an
/// open OS handle with no honest "log a copy of it" meaning (`Stdio`
/// itself is deliberately not `Clone`, see its own doc comment) — a
/// test assertion over the spawn log only ever needs to know *which*
/// wiring was requested, not the file identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdioKind {
    Inherit,
    Null,
    Pipe,
    File,
}

impl From<&Stdio> for StdioKind {
    fn from(stdio: &Stdio) -> Self {
        match stdio {
            Stdio::Inherit => StdioKind::Inherit,
            Stdio::Null => StdioKind::Null,
            Stdio::Pipe => StdioKind::Pipe,
            Stdio::File(_) => StdioKind::File,
        }
    }
}

/// A logged spawn request, for test assertions (`MockSpawner::spawned`).
/// A snapshot of [`Command`]'s shape rather than the `Command` itself:
/// `Command` is deliberately not `Clone` (a `Stdio::File` slot owns an
/// open OS handle — see `Command`'s own doc comment), so this captures
/// the same fields as an independent, `Clone`-able value, with `Stdio`
/// reduced to [`StdioKind`] (which wiring was requested, not the file
/// identity — nothing meaningful to log there anyway).
#[derive(Debug, Clone)]
pub struct SpawnRecord {
    pub program: OsString,
    pub argv: Vec<OsString>,
    pub cwd: OsString,
    pub env: EnvSpec,
    pub stdin: StdioKind,
    pub stdout: StdioKind,
    pub stderr: StdioKind,
    pub group: GroupSpec,
}

impl From<&Command> for SpawnRecord {
    fn from(cmd: &Command) -> Self {
        Self {
            program: cmd.program.clone(),
            argv: cmd.argv.clone(),
            cwd: cmd.cwd.clone(),
            env: cmd.env.clone(),
            stdin: StdioKind::from(&cmd.stdin),
            stdout: StdioKind::from(&cmd.stdout),
            stderr: StdioKind::from(&cmd.stderr),
            group: cmd.group,
        }
    }
}

/// In-memory pipe end: reads drain the buffer; writes are accepted and
/// discarded (a scripted child consumes stdin without observable effect).
struct MemPipe {
    data: Vec<u8>,
    pos: usize,
}

impl platform::fs::File for MemPipe {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn sync_all(&mut self) -> Result<()> {
        Ok(())
    }

    /// A best-effort, independent-copy clone rather than a real shared-
    /// offset duplicate (contrast `MockFile::try_clone` in `fs.rs`, which
    /// does share): `MemPipe` is this module's own private
    /// `take_stdin`/`take_stdout`/`take_stderr` representation, never
    /// constructible as a `Stdio::File` value, so the exact `dup`
    /// semantics the trait's doc comment describes are never exercised
    /// through this type.
    fn try_clone(&self) -> Result<Box<dyn platform::fs::File>> {
        Ok(Box::new(MemPipe {
            data: self.data.clone(),
            pos: self.pos,
        }))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Spawner whose children terminate exactly as scripted.
#[derive(Default)]
pub struct MockSpawner {
    scripts: BTreeMap<OsString, Script>,
    /// Log of every spawn request, for assertions.
    pub spawned: Arc<Mutex<Vec<SpawnRecord>>>,
    /// Log of every `adopt` call, for assertions — mirrors `spawned`.
    pub adopted: Arc<Mutex<Vec<u32>>>,
    /// Pids `is_alive` reports `true` for; unlisted pids report `false`
    /// (a scripted `Spawner` has no real OS process table, so "alive"
    /// is whatever the test declares it to be — the same externally-
    /// mutable-registry shape `spawned`/`adopted` already use). Tests
    /// populate this directly, e.g. after a `detach()`ed spawn whose
    /// pid they want a later `is_alive` probe to answer `true` for.
    pub live_pids: Arc<Mutex<std::collections::BTreeSet<u32>>>,
    /// Pids `is_zombie` reports `true` for; unlisted pids report
    /// `false`. Same externally-mutable-registry shape as `live_pids`.
    pub zombie_pids: Arc<Mutex<std::collections::BTreeSet<u32>>>,
}

impl MockSpawner {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn script(mut self, program: impl Into<OsString>, status: ExitStatus) -> Self {
        self.scripts.insert(
            program.into(),
            Script {
                status,
                stdout: Vec::new(),
            },
        );
        self
    }

    #[must_use]
    pub fn script_with_output(
        mut self,
        program: impl Into<OsString>,
        status: ExitStatus,
        stdout: impl Into<Vec<u8>>,
    ) -> Self {
        self.scripts.insert(
            program.into(),
            Script {
                status,
                stdout: stdout.into(),
            },
        );
        self
    }
}

struct MockChild {
    status: ExitStatus,
    own_group: bool,
    stdin: Option<Box<dyn platform::fs::File>>,
    stdout: Option<Box<dyn platform::fs::File>>,
    stderr: Option<Box<dyn platform::fs::File>>,
}

impl Child for MockChild {
    fn wait(self: Box<Self>) -> Result<ExitStatus> {
        Ok(self.status)
    }

    fn id(&self) -> u32 {
        0
    }

    /// Scripted children have already "finished"; kill succeeds without
    /// changing the scripted status, for any [`Signal`] — the mock does
    /// not model the Windows `Signal::Kill`-only restriction (divergence
    /// 008 is a backend-specific OS limitation, not a portable-contract
    /// fact). The `NewGroup`/`JoinGroup` precondition is enforced exactly
    /// like the native backends so consumer logic can be tested against
    /// it.
    fn kill_tree(&self, _sig: Signal) -> Result<()> {
        if self.own_group {
            Ok(())
        } else {
            Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "kill_tree",
            ))
        }
    }

    fn kill_single(&self, _sig: Signal) -> Result<()> {
        Ok(())
    }

    /// Scripted children have already terminated by construction.
    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        Ok(Some(self.status))
    }

    /// Scripted children never stop/continue — always the terminal
    /// scripted status, same as `try_wait`.
    fn wait_job(&mut self) -> Result<ExitStatus> {
        Ok(self.status)
    }

    fn try_wait_job(&mut self) -> Result<Option<ExitStatus>> {
        Ok(Some(self.status))
    }

    fn take_stdin(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.stdin.take()
    }

    fn take_stdout(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.stderr.take()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn mem_pipe(data: Vec<u8>) -> Box<dyn platform::fs::File> {
    Box::new(MemPipe { data, pos: 0 })
}

/// `MockSpawner::adopt`'s return type. No real OS process behind an
/// adopted pid to fail against, so both operations just succeed — same
/// "no OS limitations to model here" stance `MockChild::kill_single`
/// already takes for every `Signal` (the mock doesn't model Windows's
/// divergence-008 `Signal::Kill`-only restriction either).
struct MockGroupHandle;

impl GroupHandle for MockGroupHandle {
    fn kill_tree(&self, _sig: Signal) -> Result<()> {
        Ok(())
    }

    fn kill_single(&self, _sig: Signal) -> Result<()> {
        Ok(())
    }
}

impl Spawner for MockSpawner {
    fn spawn(&self, cmd: &Command) -> Result<Box<dyn Child>> {
        if cmd.detached && matches!(cmd.group, GroupSpec::JoinGroup(_)) {
            // Portable contract (`docs/decision-request-detach-liveness.md`):
            // a fresh session cannot also join an existing, different
            // pgid — self-contradictory on every backend, not an OS
            // limitation, so the mock enforces it too rather than only
            // the native backends.
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "spawn: detach cannot join an existing group",
            ));
        }
        crate::sync::lock(&self.spawned).push(SpawnRecord::from(cmd));
        let script = self.scripts.get(&cmd.program).ok_or_else(|| {
            PlatformError::new(ErrorKind::NotFound, OsCode::None, "spawn")
                .with_path(cmd.program.clone())
        })?;
        Ok(Box::new(MockChild {
            status: script.status,
            own_group: !matches!(cmd.group, GroupSpec::Inherit),
            stdin: matches!(cmd.stdin, Stdio::Pipe).then(|| mem_pipe(Vec::new())),
            stdout: matches!(cmd.stdout, Stdio::Pipe).then(|| mem_pipe(script.stdout.clone())),
            stderr: matches!(cmd.stderr, Stdio::Pipe).then(|| mem_pipe(Vec::new())),
        }))
    }

    fn resolve(&self, program: &OsStr) -> Result<OsString> {
        if self.scripts.contains_key(program) {
            Ok(program.to_os_string())
        } else {
            Err(PlatformError::new(ErrorKind::NotFound, OsCode::None, "resolve").with_path(program))
        }
    }

    fn adopt(&self, pid: u32) -> Result<Box<dyn GroupHandle>> {
        crate::sync::lock(&self.adopted).push(pid);
        Ok(Box::new(MockGroupHandle))
    }

    fn is_alive(&self, pid: u32) -> Result<bool> {
        Ok(crate::sync::lock(&self.live_pids).contains(&pid))
    }

    fn is_zombie(&self, pid: u32) -> Result<bool> {
        Ok(crate::sync::lock(&self.zombie_pids).contains(&pid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_child_reports_decoded_status() {
        let spawner = MockSpawner::new().script("true", ExitStatus::Code(0));
        let child = spawner.spawn(&Command::new("true", "/")).expect("spawn");
        assert!(child.wait().expect("wait").success());
    }

    #[test]
    fn wait_consumes_the_child() {
        // Compile-time property: `wait(self: Box<Self>)` means a second
        // wait cannot be written. This test documents the intent; the type
        // system is the enforcement (pins v1 scaffold bug B-4).
        let spawner = MockSpawner::new().script("x", ExitStatus::Code(1));
        let child = spawner.spawn(&Command::new("x", "/")).expect("spawn");
        let _status = child.wait().expect("wait");
        // `child.wait()` again would not compile.
    }

    #[test]
    fn kill_tree_requires_new_group() {
        let spawner = MockSpawner::new().script("x", ExitStatus::Code(0));
        let inherit = spawner.spawn(&Command::new("x", "/")).expect("spawn");
        assert_eq!(
            inherit
                .kill_tree(Signal::Kill)
                .expect_err("must refuse")
                .kind,
            ErrorKind::Unsupported
        );
        let grouped = spawner
            .spawn(&Command::new("x", "/").group(GroupSpec::NewGroup))
            .expect("spawn");
        grouped
            .kill_tree(Signal::Term)
            .expect("kill_tree with NewGroup");
        grouped
            .kill_single(Signal::Kill)
            .expect("kill_single always works");
    }

    #[test]
    fn spawn_log_supports_assertions() {
        let spawner = MockSpawner::new().script("prog", ExitStatus::Code(0));
        let log = spawner.spawned.clone();
        spawner
            .spawn(&Command::new("prog", "/work").arg("--flag"))
            .expect("spawn");
        let spawned = crate::sync::lock(&log);
        assert_eq!(spawned.len(), 1);
        assert_eq!(spawned[0].cwd, OsString::from("/work"));
    }

    #[test]
    fn adopt_succeeds_and_logs_the_pid() {
        let spawner = MockSpawner::new();
        let handle = spawner.adopt(4242).expect("adopt");
        handle.kill_tree(Signal::Kill).expect("kill_tree");
        handle.kill_single(Signal::Term).expect("kill_single");
        assert_eq!(*crate::sync::lock(&spawner.adopted), vec![4242]);
    }

    #[test]
    fn is_alive_and_is_zombie_report_whatever_the_test_declared() {
        let spawner = MockSpawner::new();
        assert!(!spawner.is_alive(99).expect("probe"));
        assert!(!spawner.is_zombie(99).expect("probe"));

        crate::sync::lock(&spawner.live_pids).insert(99);
        crate::sync::lock(&spawner.zombie_pids).insert(99);
        assert!(spawner.is_alive(99).expect("probe"));
        assert!(spawner.is_zombie(99).expect("probe"));
    }

    #[test]
    fn detach_with_join_group_is_refused() {
        let spawner = MockSpawner::new().script("true", ExitStatus::Code(0));
        let cmd = Command::new("true", "/")
            .group(GroupSpec::JoinGroup(1))
            .detach();
        let e = spawner.spawn(&cmd).err().expect("must refuse");
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }
}
