//! A block's PTY, driven by the reactor through a non-blocking master fd,
//! and the program running on it.

use std::io;
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, PlatformError};
use platform::process::{Child, Command, Signal};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use platform_linux::{LinuxChild, LinuxPty, LinuxPtyMaster};
use rusty_mux_core::Size;
use rusty_tokio::io::AsyncFd;

use crate::Config;

/// How long a closed block's program has to exit after `SIGHUP` before its
/// process group gets `SIGKILL`.
const GRACE: Duration = Duration::from_secs(1);
/// How often [`Process::reap`] polls for the exit.
const POLL: Duration = Duration::from_millis(10);

/// The master side of a block's PTY. Shared by the reader and by writers;
/// the fd closes when the last handle drops.
pub(crate) struct Pty {
    master: AsyncFd<LinuxPtyMaster>,
}

/// A block's program, owned until it is reaped. The pty child leads its own
/// session and process group, so signals go to the whole group.
pub(crate) struct Process {
    child: Option<LinuxChild>,
}

/// Starts `config`'s program on a new PTY of `size`.
pub(crate) fn spawn(config: &Config, size: Size) -> io::Result<(Pty, Process)> {
    let mut cmd = Command::new(config.program.clone(), config.cwd.clone());
    for arg in &config.args {
        cmd = cmd.arg(arg.clone());
    }
    let (master, child) = LinuxPty
        .spawn_concrete(&cmd, win_size(size))
        .map_err(to_io)?;
    // Owned before anything else can fail, so an error below still reaps it.
    let process = Process { child: Some(child) };
    master.set_nonblocking(true).map_err(to_io)?;
    let pty = Pty {
        master: AsyncFd::new(master)?,
    };
    Ok((pty, process))
}

impl Process {
    /// Hangs up the program's process group and waits for it to exit,
    /// escalating to `SIGKILL` after [`GRACE`]; a program that ignores
    /// `SIGHUP` is still reaped. Runs as its own task so the server keeps
    /// serving meanwhile.
    pub(crate) async fn reap(mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let deadline = Instant::now() + GRACE;
        let mut killed = false;
        // An unreaped leader keeps its pgid, so signalling the group before
        // `try_wait` reports the exit cannot reach an unrelated group.
        if matches!(child.try_wait(), Ok(None)) {
            signal(child, Signal::Hup);
        }
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {}
                Err(err) => {
                    // The child stays in self across awaits and errors, so
                    // cancellation also runs the kill-and-wait fallback.
                    eprintln!("rusty-mux-server: polling pid {}: {err:?}", child.id());
                    return;
                }
            }
            if !killed && Instant::now() >= deadline {
                signal(child, Signal::Kill);
                killed = true;
            }
            rusty_tokio::time::sleep(POLL).await;
        }
    }
}

fn signal(child: &LinuxChild, sig: Signal) {
    if let Err(err) = child.kill_tree(sig) {
        eprintln!("rusty-mux-server: signalling pid {}: {err:?}", child.id());
    }
}

impl Drop for Process {
    /// The fallback when a block is dropped without [`Process::reap`], for
    /// instance when the server stops: kill the group and reap it now.
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        signal(&child, Signal::Kill);
        let pid = child.id();
        if let Err(err) = Box::new(child).wait() {
            eprintln!("rusty-mux-server: reaping pid {pid}: {err:?}");
        }
    }
}

impl Pty {
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

#[cfg(test)]
mod regression {
    use super::*;
    use rusty_tokio::{Builder, LocalOptions};

    #[test]
    fn cancelled_reaper_keeps_child_ownership() {
        let rt = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .unwrap();
        rt.block_on(async {
            let cmd = Command::new("/bin/sh", "/")
                .arg("-c")
                .arg("trap '' HUP; printf ready; while :; do :; done");
            let (master, child) = LinuxPty
                .spawn_concrete(&cmd, win_size(Size::new(80, 24).unwrap()))
                .unwrap();
            let pid = child.id();
            let mut buf = [0; 64];
            assert!(master.read(&mut buf).unwrap() > 0);
            let process = Process { child: Some(child) };
            let task = rusty_tokio::task::spawn_local(process.reap());
            rusty_tokio::task::yield_now().await;
            task.abort();
            let _ = task.await;
            drop(master);
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "cancelled reaper lost child {pid}"
            );
        });
    }
}
