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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Child, Command, GroupSpec, Spawner};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;

use crate::ffi::win32_surface as w;
use crate::process::WindowsChild;
use crate::sys::handle::OwnedWinHandle;
use crate::sys::pty as syspty;

/// The Windows backend's [`Pty`] capability. Stateless, mirroring
/// [`crate::WindowsSpawner`].
pub struct WindowsPty;

impl Pty for WindowsPty {
    fn spawn(&self, cmd: &Command, size: WinSize) -> Result<(Box<dyn PtyMaster>, Box<dyn Child>)> {
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

        let (hpc, input, output) = syspty::create_pty(size)?;
        match syspty::spawn_attached(hpc, &line, &cmd.cwd, &cmd.env) {
            Ok((process, pid)) => {
                // No Job Object — see `sys::pty::spawn_attached`'s own
                // doc comment for why (a real, deliberate scope
                // reduction, not a settled design choice).
                let closed = Arc::new(AtomicBool::new(false));
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
                    Box::new(WindowsPtyMaster {
                        hpc,
                        input,
                        output,
                        closed,
                    }),
                    Box::new(child),
                ))
            }
            Err(e) => {
                // The pseudo console outlived a failed spawn attempt —
                // tear it down here rather than leaking it. Nothing was
                // ever shared with a watcher thread on this path, so a
                // direct `close` (no compare-exchange) is correct.
                syspty::close(hpc, &output);
                Err(e)
            }
        }
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
    /// Shared with the background exit-watcher thread
    /// (`syspty::spawn_exit_watcher`): guards `ClosePseudoConsole`
    /// running exactly once, whichever of "the child exits" or "the
    /// caller drops this master" happens first. The watcher never
    /// touches `output` (see its own doc comment for why), so this is
    /// the only field that needs to be shared with it.
    closed: Arc<AtomicBool>,
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
}

impl PtyMaster for WindowsPtyMaster {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        crate::sys::fileio::read(&self.output, buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        crate::sys::fileio::write(&self.input, buf)
    }

    fn resize(&self, size: WinSize) -> Result<()> {
        syspty::resize(self.hpc, size)
    }
}

impl Drop for WindowsPtyMaster {
    fn drop(&mut self) {
        // The background exit-watcher thread may have already won this
        // race (the child exited first) and already called bare
        // `ClosePseudoConsole` itself (`syspty::spawn_exit_watcher`'s own
        // doc comment) — the compare-exchange makes the real close run
        // exactly once either way. If this `Drop` wins instead (the
        // caller is giving up before the child exits), it uses the
        // draining `syspty::close`, not the watcher's bare call: this is
        // the one path where a concurrent reader can't exist to race
        // (the caller just gave up its only handle to read with), so
        // draining first — avoiding a `ClosePseudoConsole` deadlock
        // against an un-drained pipe — is both safe and necessary here
        // (`dropping_an_undrained_master_does_not_deadlock`).
        if self
            .closed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            syspty::close(self.hpc, &self.output);
        }
    }
}
