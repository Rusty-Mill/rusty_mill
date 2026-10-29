//! `Pty`/`PtyMaster` trait impls over the sys layer (RFC v2 R5+, D13,
//! convergence roadmap Phase 7, rustils#83, part 2/2 following #82's
//! Linux backend). No `unsafe` here.
//!
//! `WindowsPtyMaster` holds two separate handles (`input`: the master
//! writes here; `output`: the master reads here) rather than one, unlike
//! `LinuxPtyMaster`'s single bidirectional fd — ConPTY's master side is
//! genuinely a pair of anonymous pipes, not one descriptor. There is no
//! honest single-handle `AsHandle`/`AsRawHandle` impl to offer the way
//! `LinuxPtyMaster` ships `AsFd`/`AsRawFd`, so this type exposes two
//! named accessors instead ([`WindowsPtyMaster::input_handle`]/
//! [`WindowsPtyMaster::output_handle`]).

use std::ffi::{OsStr, OsString};
use std::sync::{Arc, Mutex, PoisonError};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Child, Command, GroupSpec, Spawner};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;

use crate::close_guard::{CloseGuard, Refused};
use crate::ffi::win32_surface as w;
use crate::process::WindowsChild;
use crate::sys::handle::OwnedWinHandle;
use crate::sys::pty as syspty;

/// The Windows backend's [`Pty`] capability. Stateless, mirroring
/// [`crate::WindowsSpawner`].
pub struct WindowsPty;

/// Serializes `CreatePseudoConsole` + `CreateProcessW` across threads.
/// This crate's own tests saw one console's output reach another when
/// consoles were created concurrently; creating them one at a time costs
/// nothing a caller would notice. [`crate::condrv::spawn_attached`] takes
/// it too, so our own host's spawns and ConPTY's never interleave.
pub(crate) static SPAWN: Mutex<()> = Mutex::new(());

impl WindowsPty {
    /// [`Pty::spawn`] with the concrete types, for callers that need
    /// [`WindowsPtyMaster::hang_up`] or `WindowsChild`'s inherent methods.
    /// Mirrors `platform-linux`'s `LinuxPty::spawn_concrete`.
    pub fn spawn_concrete(
        &self,
        cmd: &Command,
        size: WinSize,
    ) -> Result<(WindowsPtyMaster, WindowsChild)> {
        // Same contract #82's Linux backend enforces
        // (`platform::pty::Pty::spawn`'s own doc): a pty-hosted child is
        // unconditionally a fresh session/group, so there is no way to
        // also honor a request to join a different, already-existing
        // group.
        if matches!(cmd.group, GroupSpec::JoinGroup(_)) {
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "Pty::spawn: GroupSpec::JoinGroup is incompatible with a fresh pty session",
            ));
        }

        let resolved = crate::WindowsSpawner.resolve(&cmd.program)?;
        let args: Vec<&OsStr> = cmd.argv.iter().map(OsString::as_os_str).collect();
        // The security boundary: winargv classifies the resolved program
        // and builds the one command line handed to `CreateProcessW`,
        // exactly like `WindowsSpawner::spawn` already does.
        let line = crate::winargv::build_command_line(&resolved, &args)?;

        let _serial = SPAWN.lock().unwrap_or_else(PoisonError::into_inner);
        let (hpc, input, output) = syspty::create_pty(size)?;
        match syspty::spawn_attached(hpc, &line, &cmd.cwd, &cmd.env) {
            Ok((process, pid)) => {
                // No Job Object — see `sys::pty::spawn_attached`'s own
                // doc comment for why (a real, deliberate scope
                // reduction, not a settled design choice).
                let closed = Arc::new(CloseGuard::new());
                // `Ok(0)` once the child exits is this trait's own
                // documented contract (`platform::pty::PtyMaster::read`)
                // — ConPTY doesn't provide that spontaneously (see
                // `spawn_exit_watcher`'s own doc comment), so this
                // backend has to arrange it. The watcher never touches
                // `output` itself (see its own doc comment for why), so
                // only `closed` needs to be shared with it.
                syspty::spawn_exit_watcher(&process, hpc, Arc::clone(&closed));
                let child = WindowsChild::from_parts(process, None, pid);
                Ok((
                    WindowsPtyMaster {
                        hpc,
                        input,
                        output,
                        closed,
                    },
                    child,
                ))
            }
            Err(e) => {
                // The pseudo console outlived a failed spawn attempt —
                // tear it down here rather than leaking it. Nothing was
                // ever shared with a watcher thread on this path, so a
                // direct `close` (no guard) is correct.
                syspty::close(hpc, &output);
                Err(e)
            }
        }
    }
}

impl Pty for WindowsPty {
    fn spawn(&self, cmd: &Command, size: WinSize) -> Result<(Box<dyn PtyMaster>, Box<dyn Child>)> {
        let (master, child) = self.spawn_concrete(cmd, size)?;
        Ok((Box::new(master), Box::new(child)))
    }
}

/// A spawned pty pair's master side. Public for std/reactor interop (RFC
/// v2 §5.1), the same reasoning `LinuxTcpStream`/`LinuxTunDevice`/
/// `LinuxPtyMaster` document — though see this module's doc comment for
/// why the escape hatch here is two named accessors, not `AsHandle`.
pub struct WindowsPtyMaster {
    hpc: w::HPCON,
    input: OwnedWinHandle,
    output: OwnedWinHandle,
    /// `true` once `ClosePseudoConsole` has run. Shared with the
    /// background exit-watcher thread (`syspty::spawn_exit_watcher`);
    /// every close holds it across the call and [`PtyMaster::resize`]
    /// holds it across `ResizePseudoConsole`, so nothing reaches a
    /// closed console. The watcher never touches `output` (see its own
    /// doc comment for why), so this is the only field it shares.
    closed: Arc<CloseGuard>,
}

impl WindowsPtyMaster {
    /// The handle the master writes keystrokes/input to. Not pollable
    /// the way a socket handle is (anonymous pipes don't support
    /// `WaitForMultipleObjects`-style readiness signaling) — a consumer
    /// that needs that would have to bridge this onto a reactor with its
    /// own thread, the same "blocking-thread bridge" divergence D13's
    /// own text already names as expected on Windows.
    pub fn input_handle(&self) -> &OwnedWinHandle {
        &self.input
    }

    /// The handle the master reads the child's output from. Same
    /// non-pollable caveat as [`WindowsPtyMaster::input_handle`].
    pub fn output_handle(&self) -> &OwnedWinHandle {
        &self.output
    }

    /// Close the pseudo console without draining its output pipe — the
    /// exit watcher's close, on demand. A concurrent [`PtyMaster::read`]
    /// then returns `Ok(0)` once conhost has flushed, and the console's
    /// program gets Windows' console-close notification. Later
    /// [`PtyMaster::resize`] calls fail; [`PtyMaster::write`] fails once
    /// conhost has gone.
    ///
    /// Blocks while conhost is stuck writing into a full, unread output
    /// pipe, so a caller that is not also reading should run it on a
    /// thread of its own. Idempotent: a second call, or one after the
    /// program exited, returns at once.
    pub fn hang_up(&self) {
        syspty::hang_up(self.hpc, &self.closed);
    }
}

impl PtyMaster for WindowsPtyMaster {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        crate::sys::fileio::read(&self.output, buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        crate::sys::fileio::write(&self.input, buf)
    }

    /// Fails with `BrokenPipe` once the console is closed, and with
    /// `WouldBlock` while a close (or another resize) is running.
    fn resize(&self, size: WinSize) -> Result<()> {
        match self.closed.while_open(|| syspty::resize(self.hpc, size)) {
            Ok(result) => result,
            Err(Refused::Closed) => Err(PlatformError::new(
                ErrorKind::BrokenPipe,
                OsCode::None,
                "ResizePseudoConsole: the pseudo console is closed",
            )),
            Err(Refused::Busy) => Err(PlatformError::new(
                ErrorKind::WouldBlock,
                OsCode::None,
                "ResizePseudoConsole: the pseudo console is closing or being resized",
            )),
        }
    }
}

impl Drop for WindowsPtyMaster {
    fn drop(&mut self) {
        // `&mut self` rules out a concurrent `resize` or `hang_up` on this
        // master, so a busy lock means the exit watcher is inside
        // `ClosePseudoConsole`. That close may be blocked on a full,
        // unread output pipe: waiting for it here could hang forever,
        // whereas releasing our handles (which happens right after this
        // body) breaks the pipe and lets it finish.
        //
        // A free lock with the console still open means the caller is
        // giving up before the child exits. Then the draining
        // `syspty::close` runs, not the watcher's bare call: nobody can
        // be reading concurrently (the caller just gave up its only
        // handle to read with), so draining first, which keeps
        // `ClosePseudoConsole` from deadlocking against an un-drained
        // pipe, is both safe and necessary
        // (`dropping_an_undrained_master_does_not_deadlock`). Holding the
        // lock across it keeps the watcher out until it is done.
        let (hpc, output) = (self.hpc, &self.output);
        self.closed.close_unless_busy(|| syspty::close(hpc, output));
    }
}
