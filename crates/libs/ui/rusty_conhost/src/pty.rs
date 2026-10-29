//! The consumer types (design §4.2, §11.2.2): a pty-shaped front end
//! over our own console host.

use std::sync::Arc;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Child, Command, GroupSpec};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;
use platform_windows::WindowsChild;
use platform_windows::condrv;

use crate::runtime::{self, Shared};

/// Our own console host as a [`Pty`] backend. Stateless.
pub struct OwnPty;

impl OwnPty {
    /// Spawn `cmd` on a console we host, `size` being its initial window
    /// and buffer size.
    ///
    /// `Unsupported` when this Windows has no usable ConDrv: there is no
    /// silent fallback to ConPTY. `GroupSpec::JoinGroup`, or a size with
    /// 0 columns or rows, gives `InvalidInput`.
    pub fn spawn_concrete(
        &self,
        cmd: &Command,
        size: WinSize,
    ) -> Result<(OwnPtyMaster, WindowsChild)> {
        if matches!(cmd.group, GroupSpec::JoinGroup(_)) {
            return Err(invalid(
                "OwnPty::spawn: GroupSpec::JoinGroup is incompatible with a fresh console",
            ));
        }
        check_size(size, "OwnPty::spawn: the size has 0 columns or rows")?;
        condrv::probe().map_err(|e| {
            PlatformError::new(
                ErrorKind::Unsupported,
                e.os,
                "OwnPty::spawn: this Windows has no usable ConDrv",
            )
        })?;
        // The engine starts first, so once the child exists nothing can
        // fail and the caller always owns it.
        let starter = runtime::prepare(size)?;
        let attached = condrv::spawn_attached(cmd)?;
        let (shared, child) = starter.start(attached);
        Ok((OwnPtyMaster { shared }, child))
    }
}

impl Pty for OwnPty {
    fn spawn(&self, cmd: &Command, size: WinSize) -> Result<(Box<dyn PtyMaster>, Box<dyn Child>)> {
        let (master, child) = self.spawn_concrete(cmd, size)?;
        Ok((Box::new(master), Box::new(child)))
    }
}

/// The terminal's side of a console we host. `Send + Sync`.
///
/// Dropping it hangs up: the console closes on its own, and Drop never
/// waits on the engine or a client.
pub struct OwnPtyMaster {
    shared: Arc<Shared>,
}

impl OwnPtyMaster {
    /// Ask the console to close. Returns at once; idempotent.
    pub fn hang_up(&self) {
        self.shared.hang_up();
    }

    /// Whether a `read` would return without blocking: output is queued,
    /// or EOF is latched.
    pub fn output_ready(&self) -> bool {
        self.shared.output_ready()
    }

    /// A handle that waits for the direct child to exit or the console to
    /// be torn down.
    pub fn exit_waiter(&self) -> ExitWaiter {
        ExitWaiter {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl PtyMaster for OwnPtyMaster {
    /// Block until output is queued. `Ok(0)` once the console has closed
    /// and every queued byte has been read.
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.shared.read(buf)
    }

    /// Queue VT input from the terminal. Blocks only while 64 KiB of
    /// earlier input waits for the engine. `BrokenPipe` after `hang_up`
    /// or once the console has closed.
    fn write(&self, buf: &[u8]) -> Result<usize> {
        self.shared.write(buf)
    }

    /// Record the new size; the latest wins. Never blocks.
    fn resize(&self, size: WinSize) -> Result<()> {
        check_size(size, "OwnPtyMaster::resize: the size has 0 columns or rows")?;
        self.shared.resize(size)
    }
}

impl Drop for OwnPtyMaster {
    fn drop(&mut self) {
        self.shared.hang_up();
    }
}

/// Waits for the direct child to exit or the console to be torn down.
/// `Send + Sync`; any number may exist.
pub struct ExitWaiter {
    shared: Arc<Shared>,
}

impl ExitWaiter {
    /// Block until the direct child has exited or the console has been
    /// torn down, whichever is first. Returns at once if that already
    /// happened.
    pub fn wait(self) {
        self.shared.wait_exit();
    }
}

fn invalid(op: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, op)
}

fn check_size(size: WinSize, op: &'static str) -> Result<()> {
    if size.cols == 0 || size.rows == 0 {
        return Err(invalid(op));
    }
    Ok(())
}
