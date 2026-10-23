//! A block's PTY, driven by the reactor through a non-blocking master fd.

use std::io;

use platform::error::{ErrorKind, PlatformError};
use platform::process::{Child, Command};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use platform_linux::{LinuxChild, LinuxPty, LinuxPtyMaster};
use rusty_mux_core::Size;
use rusty_tokio::io::AsyncFd;

use crate::Config;

pub(crate) struct Pty {
    master: AsyncFd<LinuxPtyMaster>,
    child: LinuxChild,
}

impl Pty {
    pub(crate) fn spawn(config: &Config, size: Size) -> io::Result<Self> {
        let mut cmd = Command::new(config.program.clone(), config.cwd.clone());
        for arg in &config.args {
            cmd = cmd.arg(arg.clone());
        }
        let (master, child) = LinuxPty
            .spawn_concrete(&cmd, win_size(size))
            .map_err(to_io)?;
        master.set_nonblocking(true).map_err(to_io)?;
        Ok(Self {
            master: AsyncFd::new(master)?,
            child,
        })
    }

    /// Reads output; `Ok(0)` means the program exited.
    pub(crate) async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut ready = self.master.readable().await?;
            match ready.try_io(|fd| fd.get_ref().read(buf).map_err(to_io)) {
                Ok(Err(err)) if err.kind() == io::ErrorKind::Interrupted => continue,
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    pub(crate) async fn write_all(&self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let mut ready = self.master.writable().await?;
            match ready.try_io(|fd| fd.get_ref().write(data).map_err(to_io)) {
                Ok(Err(err)) if err.kind() == io::ErrorKind::Interrupted => continue,
                Ok(result) => data = &data[result?..],
                Err(_would_block) => continue,
            }
        }
        Ok(())
    }

    pub(crate) fn resize(&self, size: Size) -> io::Result<()> {
        self.master.get_ref().resize(win_size(size)).map_err(to_io)
    }
}

impl Drop for Pty {
    /// Closing the master hangs up the program; reap it if it already exited
    /// so it does not linger as a zombie.
    fn drop(&mut self) {
        if let Err(err) = self.child.try_wait() {
            eprintln!("rusty-mux-server: reaping pid {}: {err:?}", self.child.id());
        }
    }
}

fn win_size(size: Size) -> WinSize {
    WinSize {
        rows: size.rows(),
        cols: size.cols(),
    }
}

fn to_io(err: PlatformError) -> io::Error {
    let kind = match err.kind {
        ErrorKind::WouldBlock => io::ErrorKind::WouldBlock,
        ErrorKind::Interrupted => io::ErrorKind::Interrupted,
        ErrorKind::NotFound => io::ErrorKind::NotFound,
        ErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        ErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("{}: {:?}", err.op, err.os))
}
