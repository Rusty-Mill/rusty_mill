//! `Spawner`/`Child` impls over the sys layer (RFC v2 §5.4; extraction
//! map step 2, first slice — spawn/wait/resolve; Job-Object groups and
//! kill-tree follow with suspended spawn). No OS-level code here; the
//! command line comes exclusively from [`crate::winargv`].

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Child, Command, ExitStatus, GroupHandle, GroupSpec, Signal, Spawner};

use crate::sys::handle::OwnedWinHandle;
use crate::sys::proc;
use crate::winargv;

/// The Windows process backend.
#[derive(Debug, Default)]
pub struct WindowsSpawner;

/// A spawned child; `wait` consumes it (double-wait unrepresentable).
/// A `NewGroup` child holds its kill-on-close Job Object handle: dropping
/// the child without waiting terminates the whole tree (pinned in
/// `docs/behavior/process.md`; the disown-style detach that reverses
/// kill-on-close arrives with rush adoption — extraction map D2).
pub struct WindowsChild {
    process: OwnedWinHandle,
    job: Option<OwnedWinHandle>,
    pid: u32,
    reaped: Option<ExitStatus>,
    /// Parent pipe ends for `Stdio::Pipe` slots, until taken.
    pipes: proc::ParentPipes,
}

impl WindowsChild {
    /// Construct directly from a process handle, optional job, and pid —
    /// `pub(crate)` for `crate::pty`'s `Pty::spawn` (rustils#83), which
    /// spawns through `sys::pty::spawn_attached` rather than
    /// `sys::proc::spawn`, so has no [`proc::ParentPipes`] of its own (a
    /// pty-hosted child's stdio is the pseudo console, never a
    /// `Stdio::Pipe`) and no [`Spawner::spawn`] call site to construct
    /// one from. `job` is currently always `None` — `sys::pty` doesn't
    /// create one yet (see that module's own doc comment) — but the
    /// parameter stays `Option` rather than being dropped entirely so
    /// restoring job-based grouping later doesn't need this constructor
    /// touched again.
    pub(crate) fn from_parts(
        process: OwnedWinHandle,
        job: Option<OwnedWinHandle>,
        pid: u32,
    ) -> Self {
        Self {
            process,
            job,
            pid,
            reaped: None,
            pipes: [None, None, None],
        }
    }

    /// Disown the job object handle so that dropping this child or terminating
    /// the shell process does not kill the child process tree.
    pub fn disown(&mut self) -> Option<OwnedWinHandle> {
        self.job.take()
    }
}

impl Child for WindowsChild {
    fn wait(self: Box<Self>) -> Result<ExitStatus> {
        match self.reaped {
            Some(status) => Ok(status),
            None => proc::wait(&self.process),
        }
    }

    fn id(&self) -> u32 {
        self.pid
    }

    fn kill_tree(&self, sig: Signal) -> Result<()> {
        if sig != Signal::Kill {
            // No OS mechanism delivers an arbitrary signal identity to a
            // process here — divergence 008.
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "kill_tree",
            ));
        }
        match &self.job {
            Some(job) => proc::terminate_job(job),
            None => Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "kill_tree",
            )),
        }
    }

    fn kill_single(&self, sig: Signal) -> Result<()> {
        if sig != Signal::Kill {
            // Same divergence-008 limitation as `kill_tree`.
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "kill_single",
            ));
        }
        proc::terminate_process(&self.process)
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        // No reap semantics on Windows — the handle stays valid and the
        // exit code re-readable — but stash anyway so the trait contract
        // ("keeps reporting the same status") holds by construction.
        if self.reaped.is_none() {
            self.reaped = proc::try_wait(&self.process)?;
        }
        Ok(self.reaped)
    }

    fn wait_job(&mut self) -> Result<ExitStatus> {
        // No stop/continue analog on Windows (D8): job-control suspend
        // is a Unix-only concept (no `tcsetpgrp`/SIGTSTP equivalent).
        Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "wait_job",
        ))
    }

    fn try_wait_job(&mut self) -> Result<Option<ExitStatus>> {
        Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "try_wait_job",
        ))
    }

    fn take_stdin(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.pipes[0]
            .take()
            .map(|h| Box::new(crate::fs::WindowsFile::from(h)) as Box<dyn platform::fs::File>)
    }

    fn take_stdout(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.pipes[1]
            .take()
            .map(|h| Box::new(crate::fs::WindowsFile::from(h)) as Box<dyn platform::fs::File>)
    }

    fn take_stderr(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.pipes[2]
            .take()
            .map(|h| Box::new(crate::fs::WindowsFile::from(h)) as Box<dyn platform::fs::File>)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A kill-scoped handle over an adopted, externally-spawned pid
/// (`Spawner::adopt`, rustils#47). Always holds a job — unlike
/// `WindowsChild`, where `job` is `Option` for an ungrouped spawn,
/// `adopt` always creates one: giving kill-tree semantics to a pid this
/// backend didn't spawn is the entire point of adopting it.
pub struct WindowsGroupHandle {
    process: OwnedWinHandle,
    job: OwnedWinHandle,
}

impl GroupHandle for WindowsGroupHandle {
    fn kill_tree(&self, sig: Signal) -> Result<()> {
        if sig != Signal::Kill {
            // Same divergence-008 limitation as `WindowsChild::kill_tree`.
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "kill_tree",
            ));
        }
        proc::terminate_job(&self.job)
    }

    fn kill_single(&self, sig: Signal) -> Result<()> {
        if sig != Signal::Kill {
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "kill_single",
            ));
        }
        proc::terminate_process(&self.process)
    }
}

/// The default PATHEXT set, per the OS default when the variable is
/// absent.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

fn has_extension(program: &OsStr) -> bool {
    Path::new(program)
        .extension()
        .is_some_and(|e| !e.is_empty())
}

fn candidate_if_file(path: PathBuf) -> Option<PathBuf> {
    std::fs::metadata(&path)
        .map(|m| m.is_file())
        .unwrap_or(false)
        .then_some(path)
}

/// Try `base` as given (if it already has an extension), then with each
/// PATHEXT suffix.
fn resolve_with_pathext(base: &Path, pathext: &OsStr) -> Option<PathBuf> {
    if has_extension(base.as_os_str()) {
        if let Some(hit) = candidate_if_file(base.to_path_buf()) {
            return Some(hit);
        }
    }
    let exts = pathext.to_string_lossy();
    for ext in exts.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let mut with_ext = base.as_os_str().to_os_string();
        with_ext.push(ext);
        if let Some(hit) = candidate_if_file(PathBuf::from(with_ext)) {
            return Some(hit);
        }
    }
    None
}

impl Spawner for WindowsSpawner {
    fn spawn(&self, cmd: &Command) -> Result<Box<dyn Child>> {
        if matches!(cmd.group, GroupSpec::JoinGroup(_)) {
            // Windows has no numeric process-group id a spawn can join —
            // Job Objects are handle-based, with no equivalent to
            // POSIX's "start this child already inside pgid N"
            // (divergence 008). Refuse before spawning anything rather
            // than silently falling back to `Inherit`/`NewGroup`.
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "spawn: GroupSpec::JoinGroup",
            ));
        }
        if cmd.detached && cmd.group == GroupSpec::NewGroup {
            // `Command::detach`'s contract (`docs/decision-request-
            // detach-liveness.md`): a kill-on-close Job Object is torn
            // down — killing every member — the instant every handle to
            // it closes, which the OS does unconditionally when this
            // process terminates for any reason including a crash.
            // Combined with `detach`, that would silently defeat the
            // very "survives even a crashed parent" guarantee `detach`
            // promises. Refused rather than silently dropping one of the
            // two requests: a caller that wants both a detached child
            // and a later kill_tree on it uses `detach()` alone at
            // spawn, then `Spawner::adopt` on its pid when it actually
            // decides to kill it.
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "spawn: detach with GroupSpec::NewGroup",
            ));
        }
        let resolved = self.resolve(&cmd.program)?;
        let args: Vec<&OsStr> = cmd.argv.iter().map(OsString::as_os_str).collect();
        // The security boundary: winargv classifies the resolved program
        // (.bat/.cmd get cmd-rules quoting or refusal) and builds the one
        // command line handed to CreateProcessW.
        let line = winargv::build_command_line(&resolved, &args)?;
        let (process, job, pid, pipes) = proc::spawn(
            &line,
            &cmd.cwd,
            &cmd.env,
            [&cmd.stdin, &cmd.stdout, &cmd.stderr],
            cmd.group == GroupSpec::NewGroup,
            cmd.detached,
        )?;
        Ok(Box::new(WindowsChild {
            process,
            job,
            pid,
            reaped: None,
            pipes,
        }))
    }

    /// Mechanism-level lookup (RFC v2 §5.4): a name containing a path
    /// separator is tried directly (with PATHEXT completion if it lacks
    /// an extension); a bare name is searched through `PATH`, each entry
    /// tried with PATHEXT. Policy layers live in consumers.
    fn resolve(&self, program: &OsStr) -> Result<OsString> {
        if program.is_empty() {
            return Err(
                PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "resolve")
                    .with_path(program),
            );
        }
        let pathext =
            std::env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(DEFAULT_PATHEXT));
        let is_pathy =
            program.to_string_lossy().contains(['\\', '/']) || Path::new(program).is_absolute();
        if is_pathy {
            if let Some(hit) = resolve_with_pathext(Path::new(program), &pathext) {
                return Ok(hit.into_os_string());
            }
            return Err(
                PlatformError::new(ErrorKind::NotFound, OsCode::None, "resolve").with_path(program),
            );
        }
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                if dir.as_os_str().is_empty() {
                    continue;
                }
                if let Some(hit) = resolve_with_pathext(&dir.join(program), &pathext) {
                    return Ok(hit.into_os_string());
                }
            }
        }
        Err(PlatformError::new(ErrorKind::NotFound, OsCode::None, "resolve").with_path(program))
    }

    /// R3 reactor internals (RFC v2 §5.6): `WaitForMultipleObjects` with
    /// the 64-handle cap absorbed in `sys::proc::wait_many`. Same
    /// contract as `process::wait_any`; falls back to the portable loop
    /// for children that aren't ours.
    fn wait_any(
        &self,
        children: &mut [Box<dyn Child>],
        timeout: Option<std::time::Duration>,
    ) -> Result<Option<usize>> {
        if children.is_empty() {
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "wait_any",
            ));
        }
        // Fast pass — an already-terminated child wins without the OS
        // multiplexer; also the downcast gate for the native path.
        let mut all_ours = true;
        for (i, child) in children.iter_mut().enumerate() {
            if child.try_wait()?.is_some() {
                return Ok(Some(i));
            }
            if child.as_any_mut().downcast_mut::<WindowsChild>().is_none() {
                all_ours = false;
                break;
            }
        }
        if !all_ours {
            return platform::process::wait_any(children, timeout);
        }
        // Raw handle values, one pass; the `&mut children` this method
        // holds keeps every handle open across the wait (the contract
        // `sys::proc::wait_many` documents).
        let mut handles: Vec<_> = Vec::with_capacity(children.len());
        for child in children.iter_mut() {
            let native = child
                .as_any_mut()
                .downcast_mut::<WindowsChild>()
                .ok_or_else(|| {
                    PlatformError::new(
                        ErrorKind::Other,
                        OsCode::None,
                        "wait_any: child type changed between the gating pass and the native pass",
                    )
                })?;
            handles.push(native.process.as_raw());
        }
        match proc::wait_many(&handles, timeout)? {
            Some(index) => {
                children[index].try_wait()?;
                Ok(Some(index))
            }
            None => Ok(None),
        }
    }

    fn adopt(&self, pid: u32) -> Result<Box<dyn GroupHandle>> {
        let (process, job) = proc::adopt(pid)?;
        Ok(Box::new(WindowsGroupHandle { process, job }))
    }

    fn is_alive(&self, pid: u32) -> Result<bool> {
        proc::is_alive(pid)
    }

    fn is_zombie(&self, _pid: u32) -> Result<bool> {
        // No zombie concept on Windows — a process handle stays valid
        // and its exit code re-readable indefinitely after exit, with no
        // distinct "exited but unreaped" state to observe (divergence
        // 016).
        Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "is_zombie",
        ))
    }
}
