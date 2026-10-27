//! The TUI paints a short-lived child's last output before it exits (#1162).
//! Runs the real binary on a fresh pty, as the host terminal, and reads what
//! it painted there.
#![cfg(unix)]

use std::fs::File;
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A pty pair sized 80x10: (master, slave).
fn openpty() -> (OwnedFd, OwnedFd) {
    let (mut master, mut slave) = (-1, -1);
    let ws = libc::winsize {
        ws_row: 10,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: valid out-pointers; a null name and termios are allowed.
    let r = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &ws,
        )
    };
    assert_eq!(r, 0, "openpty: {}", std::io::Error::last_os_error());
    // openpty's fds are inheritable: without close-on-exec they leak through
    // rusty_term into its child, and a grandchild then holds the pty open.
    for fd in [master, slave] {
        // SAFETY: `fd` is open; F_SETFD only changes its flags.
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) }, 0);
    }
    // SAFETY: openpty returned two fresh fds that nothing else owns.
    unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

/// Everything `rusty_term -- <cmd>` writes to its host terminal, read until
/// the pty closes.
fn host_output(cmd: &str) -> (String, Duration) {
    let (master, slave) = openpty();
    let start = Instant::now();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty_term"));
    command
        .args(["--", "sh", "-c", cmd])
        .env("RUSTY_TERM_CONFIG", "/dev/null")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    // SAFETY: only async-signal-safe calls between fork and exec. The slave
    // becomes the controlling terminal, so the TUI's `/dev/tty` opens it.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    drop(command); // closes our copies of the slave, so the master EOFs
    let mut master = File::from(master);
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    // EIO once the last slave fd closes: the TUI has exited.
    while let Ok(n) = master.read(&mut buf) {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    let _ = child.wait();
    (String::from_utf8_lossy(&out).into_owned(), start.elapsed())
}

#[test]
fn last_frame_is_painted_when_a_short_lived_child_exits() {
    for run in 0..20 {
        let (out, _) = host_output("echo FIRST; echo LAST");
        assert!(out.contains("LAST"), "run {run}: LAST not painted: {out:?}");
    }
}

#[test]
fn exit_is_prompt_when_a_grandchild_holds_the_pty() {
    // The backgrounded sleep ignores the hangup sent when the shell exits, so
    // it keeps the pty's slave open and EOF never comes while it runs.
    // Shutdown waits out the drain bound, then paints and exits long before
    // the grandchild does.
    let (out, elapsed) = host_output("trap '' HUP; sleep 3 & echo LAST");
    assert!(out.contains("LAST"), "LAST not painted: {out:?}");
    assert!(elapsed >= Duration::from_millis(250), "no drain wait: {elapsed:?}");
    assert!(elapsed < Duration::from_millis(2500), "took {elapsed:?}");
}

#[test]
fn final_frame_releases_a_synchronized_update_left_open() {
    // The child opens a synchronized update (DEC 2026) and exits inside it:
    // its output is still painted.
    let (out, _) = host_output("printf '\\033[?2026hLAST'");
    assert!(out.contains("LAST"), "LAST not painted: {out:?}");
}
