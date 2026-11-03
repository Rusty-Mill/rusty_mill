//! Linux: a block's PTY, driven by the reactor through a non-blocking
//! master fd, and the program running on it.

use std::ffi::OsString;
use std::io;
use std::os::fd::AsRawFd;
use std::time::Instant;

use platform::process::{Child, Signal};
use platform::pty::PtyMaster;
use platform_linux::{LinuxChild, LinuxPty, LinuxPtyMaster};
use rusty_mux_core::{ExitStatus, Size};
use rusty_mux_proto::Echo;
use rusty_tokio::io::AsyncFd;

use super::{GRACE, POLL, command, exit_status, to_io, win_size};
use crate::Config;

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

/// Starts `config`'s program on a new PTY of `size`, with this process's
/// environment plus `env`.
pub(crate) fn spawn(
    config: &Config,
    size: Size,
    env: &[(&str, OsString)],
) -> io::Result<(Pty, Process)> {
    let cmd = command(config, env);
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
    /// The program's process id, until it is reaped.
    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|child| child.id())
    }

    /// Hangs up the program's process group and waits for it to exit,
    /// escalating to `SIGKILL` after [`GRACE`]; a program that ignores
    /// `SIGHUP` is still reaped. Runs as its own task so the server keeps
    /// serving meanwhile.
    /// How it ended, or `None` if that can't be known.
    pub(crate) async fn reap(mut self) -> Option<ExitStatus> {
        let child = self.child.as_mut()?;
        let deadline = Instant::now() + GRACE;
        let mut killed = false;
        // An unreaped leader keeps its pgid, so signalling the group before
        // `try_wait` reports the exit cannot reach an unrelated group.
        if matches!(child.try_wait(), Ok(None)) {
            signal(child, Signal::Hup);
        }
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return exit_status(status),
                Ok(None) => {}
                Err(err) => {
                    // The child stays in self across awaits and errors, so
                    // cancellation also runs the kill-and-wait fallback.
                    eprintln!("rusty-mux-server: polling pid {}: {err:?}", child.id());
                    return None;
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

/// Nothing to wait for when the server stops: the kernel hangs up a
/// block's PTY with its last master, and the dropped [`Process`] has
/// killed and reaped the program's group.
#[derive(Default)]
pub(crate) struct Hosts;

impl Hosts {
    pub(crate) fn track(&mut self, _process: &Process) {}

    pub(crate) async fn shut_down(self, _budget: std::time::Duration) -> bool {
        true
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
    /// Reads output; `Ok(0)` means the program exited. Cancelling it loses
    /// nothing.
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

    /// Reads output without waiting; `None` means the master held none, so
    /// every byte the program wrote before this call has been read.
    pub(crate) fn try_read(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        match self.master.get_ref().read(buf).map_err(to_io) {
            Ok(n) => Ok(Some(n)),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(err),
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

    /// The terminal's foreground process group, `None` once the program
    /// side is gone.
    pub(crate) fn foreground(&self) -> Option<u32> {
        let fd = self.master.get_ref().as_raw_fd();
        let pgrp = rusty_libc::termios::tcgetpgrp(fd).ok()?;
        u32::try_from(pgrp).ok().filter(|&p| p > 0)
    }

    /// Who echoes typing, from the program side's termios, which the master
    /// reports.
    pub(crate) fn echo(&self) -> Echo {
        use rusty_libc::termios::{ECHO, ICANON, tcgetattr};
        let Ok(termios) = tcgetattr(self.master.get_ref().as_raw_fd()) else {
            return Echo::Unknown;
        };
        let flags = termios.c_lflag;
        match (flags & ECHO != 0, flags & ICANON != 0) {
            (true, _) => Echo::Kernel,
            (false, true) => Echo::Hidden,
            (false, false) => Echo::Program,
        }
    }

    /// Nothing to do on Linux: the fd closes with the last handle, and an
    /// unread PTY only stalls its own program.
    pub(crate) fn release(&self) {}
}

#[cfg(test)]
mod regression {
    use super::*;
    use platform::process::Command;
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
