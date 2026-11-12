//! Linux: a block's PTY, driven by the reactor through a non-blocking
//! master fd, and the program running on it.

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::time::Instant;

use platform::process::{Child, Signal};
use platform::pty::PtyMaster;
use platform_linux::{LinuxChild, LinuxPty, LinuxPtyMaster};
use rusty_mux_core::{ExitStatus, Size};
use rusty_mux_proto::{BlockSignal, Echo};
use rusty_tokio::io::AsyncFd;

use super::{Env, GRACE, POLL, command, exit_status, to_io, win_size};
use crate::Config;
use crate::passed::Fd;

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

/// Starts `config`'s program on a new PTY of `size`, with the environment
/// `env` gives and `passed` at descriptors 3, 4, and on (C75), which close
/// here once it has started or failed to. `owned` matters only on Windows:
/// the program leads its own session and process group here either way.
pub(crate) fn spawn(
    config: &Config,
    size: Size,
    env: Env<'_>,
    _owned: bool,
    passed: Vec<Fd>,
) -> io::Result<(Pty, Process)> {
    let cmd = command(config, env);
    let borrowed: Vec<BorrowedFd<'_>> = passed.iter().map(AsFd::as_fd).collect();
    let (master, child) = LinuxPty
        .spawn_passing(&cmd, win_size(size), &borrowed)
        .map_err(to_io)?;
    drop(borrowed);
    drop(passed);
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

impl Process {
    /// Sends `sig` to the program's process group (C76).
    pub(crate) fn signal(&self, sig: BlockSignal) -> io::Result<()> {
        // Gone already: say so rather than report a signal that went nowhere.
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| io::Error::other("exited"))?;
        child.kill_tree(platform_signal(sig)).map_err(to_io)
    }

    /// Hands the program over to a [`Released`] waiter, which never kills
    /// it (C76). Synchronous, so the kill-on-drop is disarmed before any
    /// future exists that could be dropped unpolled.
    pub(crate) fn release(mut self) -> Option<Released> {
        self.child.take().map(|child| Released { child })
    }

    /// Hangs up the program's group and leaves it running, unreaped: a
    /// stopping server does this to an owned block, never killing it
    /// (D87). Whoever owns the block cleans up.
    pub(crate) fn abandon(mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(None)) {
            signal(&child, Signal::Hup);
        }
    }
}

/// A released block's program (C76): waited for, never signalled. Dropping
/// it, polled or not, leaves the program running for its owner to end.
pub(crate) struct Released {
    child: LinuxChild,
}

impl Released {
    /// Waits for the program to exit. How it ended, or `None` if that
    /// can't be known.
    pub(crate) async fn wait(mut self) -> Option<ExitStatus> {
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return exit_status(status),
                Ok(None) => rusty_tokio::time::sleep(POLL).await,
                Err(err) => {
                    eprintln!("rusty-mux-server: polling pid {}: {err:?}", self.child.id());
                    return None;
                }
            }
        }
    }
}

fn platform_signal(sig: BlockSignal) -> Signal {
    match sig {
        BlockSignal::Hup => Signal::Hup,
        BlockSignal::Int => Signal::Int,
        BlockSignal::Kill => Signal::Kill,
        BlockSignal::Term => Signal::Term,
        BlockSignal::Cont => Signal::Cont,
        BlockSignal::Stop => Signal::Stop,
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

    pub(crate) async fn write_all(&self, data: &[u8]) -> io::Result<()> {
        self.write_counted(data, &std::cell::Cell::new(0)).await
    }

    /// [`Self::write_all`], adding to `written` as the program takes each
    /// part, so a caller that abandons the write knows how much it got.
    pub(crate) async fn write_counted(
        &self,
        mut data: &[u8],
        written: &std::cell::Cell<usize>,
    ) -> io::Result<()> {
        while !data.is_empty() {
            let mut ready = self.master.writable().await?;
            match ready.try_io(|fd| fd.get_ref().write(data).map_err(to_io)) {
                Ok(Err(err)) if err.kind() == io::ErrorKind::Interrupted => continue,
                Ok(result) => {
                    let n = result?;
                    written.set(written.get() + n);
                    data = &data[n..];
                }
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

    /// The block's processes for `terminal.process`: the foreground
    /// process and its ancestors, then its program `child`.
    pub(crate) fn processes(&self, child: u32) -> Vec<crate::process::Proc> {
        crate::process::chain(child, self.foreground(), crate::process::lookup)
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

    /// A HUP-ignoring program on a fresh PTY, its pid, and the master.
    fn stubborn() -> (Process, u32, LinuxPtyMaster) {
        let cmd = Command::new("/bin/sh", "/")
            .arg("-c")
            .arg("trap '' HUP; printf ready; while :; do sleep 0.05; done");
        let (master, child) = LinuxPty
            .spawn_concrete(&cmd, win_size(Size::new(80, 24).unwrap()))
            .unwrap();
        let mut buf = [0; 64];
        assert!(master.read(&mut buf).unwrap() > 0);
        let pid = child.id();
        (Process { child: Some(child) }, pid, master)
    }

    fn alive(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| stat.rsplit(')').next()?.trim().chars().next())
            .is_some_and(|state| !matches!(state, 'Z' | 'X'))
    }

    /// Ends and reaps what a test left running.
    fn finish(pid: u32) {
        let pid = i32::try_from(pid).unwrap();
        let _ = rusty_libc::process::kill(pid, rusty_libc::signal::SIGKILL);
        let _ = rusty_libc::wait::waitpid(pid, 0);
    }

    #[test]
    fn a_release_never_kills_however_its_waiter_ends() {
        let rt = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .unwrap();
        rt.block_on(async {
            // Dropped without ever being polled.
            let (process, pid, master) = stubborn();
            drop(process.release().expect("a child").wait());
            drop(master);
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(alive(pid), "an unpolled waiter killed {pid}");
            finish(pid);

            // Spawned and aborted before its first poll.
            let (process, pid, master) = stubborn();
            let released = process.release().expect("a child");
            let task = rusty_tokio::task::spawn_local(released.wait());
            task.abort();
            let _ = task.await;
            drop(master);
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(alive(pid), "an aborted unpolled waiter killed {pid}");
            finish(pid);

            // Aborted after it has polled.
            let (process, pid, master) = stubborn();
            let task = rusty_tokio::task::spawn_local(process.release().expect("a child").wait());
            rusty_tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            task.abort();
            let _ = task.await;
            drop(master);
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(alive(pid), "an aborted waiter killed {pid}");
            finish(pid);
        });
    }

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
