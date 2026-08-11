//! `Spawner`/`Child` impls over the sys layer (RFC v2 §5.4; extraction
//! map step 2, first slice — spawn/wait/resolve; groups and kill-tree
//! follow). No `unsafe` here.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Child, Command, ExitStatus, GroupHandle, GroupSpec, Signal, Spawner};

use crate::ffi::libc_surface as c;
use crate::sys::spawn;

/// The Linux process backend.
#[derive(Debug, Default)]
pub struct LinuxSpawner;

/// A spawned child; `wait` consumes it (double-wait unrepresentable).
pub struct LinuxChild {
    pid: c::pid_t,
    /// The group [`Child::kill_tree`] targets: `Some(pid)` for
    /// `GroupSpec::NewGroup` (this child leads it — pid == pgid) or
    /// `GroupSpec::JoinGroup(pgid)` (this child joined an existing one —
    /// the explicit target, not this child's own pid); `None` for
    /// `GroupSpec::Inherit`, where `kill_tree` has no sound target.
    group: Option<c::pid_t>,
    /// Set by a successful `try_wait`: `WNOHANG` reaps the zombie, so the
    /// decoded status must be stashed for the eventual consuming `wait`.
    reaped: Option<ExitStatus>,
    /// Parent pipe ends for `Stdio::Pipe` slots, until taken.
    pipes: spawn::ParentPipes,
}

impl LinuxChild {
    /// Construct directly from a pid and its group target — `pub(crate)`
    /// for `crate::pty`'s `Pty::spawn` (rustils#82), which spawns through
    /// `sys::pty::spawn_attached` rather than `sys::spawn::spawn`, so has
    /// no [`spawn::ParentPipes`] of its own (a pty-hosted child's stdio is
    /// the slave, never a `Stdio::Pipe`) and no [`Spawner::spawn`] call
    /// site to construct one from.
    pub(crate) fn from_pid(pid: c::pid_t, group: Option<c::pid_t>) -> Self {
        Self {
            pid,
            group,
            reaped: None,
            pipes: [None, None, None],
        }
    }
}

impl Child for LinuxChild {
    fn wait(self: Box<Self>) -> Result<ExitStatus> {
        match self.reaped {
            Some(status) => Ok(status),
            None => spawn::wait(self.pid),
        }
    }

    fn id(&self) -> u32 {
        self.pid as u32
    }

    fn kill_tree(&self, sig: Signal) -> Result<()> {
        match self.group {
            // Killing the parent's own group is the only alternative
            // target and never what the caller meant (trait contract).
            None => Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "kill_tree",
            )),
            Some(pgid) => spawn::kill_group(pgid, sig),
        }
    }

    fn kill_single(&self, sig: Signal) -> Result<()> {
        spawn::kill_single(self.pid, sig)
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        if self.reaped.is_none() {
            self.reaped = spawn::try_wait(self.pid)?;
        }
        Ok(self.reaped)
    }

    fn wait_job(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.reaped {
            return Ok(status);
        }
        let status = spawn::wait_job(self.pid)?;
        if !matches!(status, ExitStatus::Stopped(_) | ExitStatus::Continued) {
            self.reaped = Some(status);
        }
        Ok(status)
    }

    fn try_wait_job(&mut self) -> Result<Option<ExitStatus>> {
        if let Some(status) = self.reaped {
            return Ok(Some(status));
        }
        let status = spawn::try_wait_job(self.pid)?;
        if let Some(s) = status {
            if !matches!(s, ExitStatus::Stopped(_) | ExitStatus::Continued) {
                self.reaped = Some(s);
            }
        }
        Ok(status)
    }

    fn take_stdin(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.pipes[0]
            .take()
            .map(|fd| Box::new(crate::fs::LinuxFile::from(fd)) as Box<dyn platform::fs::File>)
    }

    fn take_stdout(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.pipes[1]
            .take()
            .map(|fd| Box::new(crate::fs::LinuxFile::from(fd)) as Box<dyn platform::fs::File>)
    }

    fn take_stderr(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.pipes[2]
            .take()
            .map(|fd| Box::new(crate::fs::LinuxFile::from(fd)) as Box<dyn platform::fs::File>)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

impl Spawner for LinuxSpawner {
    fn spawn(&self, cmd: &Command) -> Result<Box<dyn Child>> {
        if cmd.detached && cmd.group != GroupSpec::Inherit {
            // `Command::detach`'s contract (`docs/decision-request-
            // detach-liveness.md`): `JoinGroup` is refused everywhere —
            // `setsid` starting a brand-new session cannot also join an
            // existing, different pgid, self-contradictory rather than an
            // OS limitation. `NewGroup` is *also* refused here — not for
            // that reason, but a real kernel one found by an actual
            // `posix_spawn` call returning `EPERM`: Linux's `setpgid(2)`
            // explicitly forbids changing the process group ID of a
            // session leader, full stop, even a self-targeting
            // `setpgid(0, 0)` no-op (`man 2 setpgid`: "An attempt to
            // change the process group ID of a session leader fails with
            // EPERM"). `setsid` (which `detached` sets) always makes the
            // child a session leader, so `POSIX_SPAWN_SETPGROUP` can never
            // follow it — refused up front rather than surfacing the raw
            // kernel `EPERM` from a doomed `posix_spawn` call.
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "spawn: detach with a non-Inherit GroupSpec",
            ));
        }
        let resolved = self.resolve(&cmd.program)?;
        let (pid, pipes) = spawn::spawn(
            &resolved,
            &cmd.program,
            &cmd.argv,
            &cmd.cwd,
            &cmd.env,
            [&cmd.stdin, &cmd.stdout, &cmd.stderr],
            cmd.group,
            cmd.detached,
        )?;
        let group = match cmd.group {
            // `detached` alone already gives the child its own pgid via
            // `setsid` (`sys::spawn::spawn`'s doc comment) — `kill_tree`
            // gets a sound target even without an explicit `NewGroup`.
            GroupSpec::Inherit if cmd.detached => Some(pid),
            GroupSpec::Inherit => None,
            // A fresh group's pgid IS the leader's own pid (pgroup 0 at
            // spawn means exactly that).
            GroupSpec::NewGroup => Some(pid),
            GroupSpec::JoinGroup(pgid) => Some(pgid as c::pid_t),
        };
        Ok(Box::new(LinuxChild {
            pid,
            group,
            reaped: None,
            pipes,
        }))
    }

    /// Mechanism-level lookup (RFC v2 §5.4): a name containing `/` is used
    /// as-is; otherwise each `$PATH` entry is searched for a regular file
    /// with any execute bit. Policy layers (builtin precedence, shebang
    /// emulation) live in consumers.
    fn resolve(&self, program: &OsStr) -> Result<OsString> {
        use std::os::unix::ffi::OsStrExt;
        if program.is_empty() {
            return Err(
                PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "resolve")
                    .with_path(program),
            );
        }
        if program.as_bytes().contains(&b'/') {
            let p = Path::new(program);
            if is_executable_file(p) {
                return Ok(program.to_os_string());
            }
            return Err(
                PlatformError::new(ErrorKind::NotFound, OsCode::None, "resolve").with_path(program),
            );
        }
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate: PathBuf = dir.join(program);
                if is_executable_file(&candidate) {
                    return Ok(candidate.into_os_string());
                }
            }
        }
        Err(PlatformError::new(ErrorKind::NotFound, OsCode::None, "resolve").with_path(program))
    }

    /// R3 reactor internals (RFC v2 §5.6): pidfd + `poll` instead of the
    /// portable tick loop. Same contract as `process::wait_any`; falls
    /// back to the portable loop when a child isn't ours or the kernel
    /// lacks pidfd_open.
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
        // Fast pass — an already-terminated (or already-stashed) child
        // wins without touching the OS multiplexer; also the downcast
        // gate for the native path.
        let mut pending: Vec<(usize, c::pid_t)> = Vec::with_capacity(children.len());
        let mut all_ours = true;
        for (i, child) in children.iter_mut().enumerate() {
            if child.try_wait()?.is_some() {
                return Ok(Some(i));
            }
            match child.as_any_mut().downcast_mut::<LinuxChild>() {
                Some(c) => pending.push((i, c.pid)),
                None => {
                    all_ours = false;
                    break;
                }
            }
        }
        if !all_ours {
            return platform::process::wait_any(children, timeout);
        }
        let pids: Vec<c::pid_t> = pending.iter().map(|&(_, pid)| pid).collect();
        match spawn::poll_pids(&pids, timeout) {
            Ok(Some(pos)) => {
                let (index, _) = pending[pos];
                // Reap now so the winner's status is stashed (the
                // wait_any contract: retrieve via try_wait/wait).
                children[index].try_wait()?;
                Ok(Some(index))
            }
            Ok(None) => Ok(None),
            Err(e) if e.kind == ErrorKind::Unsupported => {
                platform::process::wait_any(children, timeout)
            }
            Err(e) => Err(e),
        }
    }

    fn adopt(&self, _pid: u32) -> Result<Box<dyn GroupHandle>> {
        // POSIX `setpgid(pid, pgid)` can only retarget a process that is
        // both the caller's own child *and* has not yet exec'd — by the
        // time any caller has a pid to adopt (e.g. from
        // `portable-pty::Child::process_id()`), the target has already
        // exec'd, so there is no sound way to honor this on Unix
        // (divergence 010). Never attempted speculatively: a `setpgid`
        // that sometimes works depending on timing would be worse than
        // an honest refusal.
        Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "adopt",
        ))
    }

    fn is_alive(&self, pid: u32) -> Result<bool> {
        spawn::is_alive(pid as c::pid_t)
    }

    fn is_zombie(&self, pid: u32) -> Result<bool> {
        spawn::is_zombie(pid as c::pid_t)
    }
}
