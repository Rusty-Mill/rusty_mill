//! `Pty`/`PtyMaster` trait impls over the sys layer (RFC v2 R5+, D13,
//! convergence roadmap Phase 7, rustils#82). No `unsafe` here.
//!
//! `LinuxPtyMaster` also gets `AsFd`/`AsRawFd` — the same raw-fd escape
//! hatch `LinuxTcpStream`/`LinuxTunDevice` ship (rustils#41/#42, Tun's own
//! Phase 8 precedent), since the master fd is a real, pollable fd here
//! that a consumer building its own reactor may need to register directly
//! rather than driving I/O through the object-safe trait's blocking calls.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Child, Command, GroupSpec, Spawner};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;

use crate::process::LinuxChild;
use crate::sys::pty as syspty;

/// The Linux backend's [`Pty`] capability. Stateless, mirroring
/// [`crate::LinuxSpawner`]/[`crate::LinuxTun`].
pub struct LinuxPty;

impl Pty for LinuxPty {
    fn spawn(&self, cmd: &Command, size: WinSize) -> Result<(Box<dyn PtyMaster>, Box<dyn Child>)> {
        // A pty-hosted child is unconditionally a new session leader
        // (that's the mechanism that gives it the slave as its
        // controlling terminal) — which makes it a fresh process-group
        // leader too, by definition. There's no way to also honor a
        // request to join a different, already-existing group (trait
        // contract, `platform::pty::Pty::spawn`'s own doc).
        if matches!(cmd.group, GroupSpec::JoinGroup(_)) {
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "Pty::spawn: GroupSpec::JoinGroup is incompatible with a fresh pty session",
            ));
        }

        let resolved = crate::LinuxSpawner.resolve(&cmd.program)?;
        let (master, slave_path) = syspty::open_pty_pair()?;
        let pid = syspty::spawn_attached(
            &resolved,
            &cmd.program,
            &cmd.argv,
            &cmd.cwd,
            &cmd.env,
            &slave_path,
        )?;
        syspty::resize(&master, size)?;

        // The child is its own session leader, so its pgid == its own
        // pid — the same target `GroupSpec::NewGroup` gets in the plain
        // `Spawner::spawn` path (`LinuxSpawner::spawn`'s own comment).
        let child = LinuxChild::from_pid(pid, Some(pid));
        Ok((Box::new(LinuxPtyMaster { master }), Box::new(child)))
    }
}

/// A spawned pty pair's master side. Public for std/reactor interop (RFC
/// v2 §5.1), the same reasoning `LinuxTcpStream`/`LinuxTunDevice` document.
pub struct LinuxPtyMaster {
    master: OwnedFd,
}

impl PtyMaster for LinuxPtyMaster {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        syspty::read(&self.master, buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        syspty::write(&self.master, buf)
    }

    fn resize(&self, size: WinSize) -> Result<()> {
        syspty::resize(&self.master, size)
    }
}

impl AsFd for LinuxPtyMaster {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.master.as_fd()
    }
}

impl AsRawFd for LinuxPtyMaster {
    fn as_raw_fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }
}
