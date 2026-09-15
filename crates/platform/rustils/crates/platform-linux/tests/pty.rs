//! Live PTY test (rustils#82): spawns real children attached to a real
//! kernel pty pair and exercises the actual protocol, not a mock — the
//! same live-verification bar #77/#78's D-Bus/Secret Service work was
//! held to, and for the same reason: the `posix_spawn`-native substitute
//! for `fork`+`TIOCSCTTY` (`docs/design-discussion-pty.md`) is exactly
//! the kind of thing that looks right by inspection and is wrong in
//! practice if the file-actions/attribute ordering isn't what's assumed.
//!
//! `session_and_controlling_terminal_are_actually_assigned` reads the
//! spawned child's own `/proc/<pid>/stat` — kernel ground truth, not
//! this crate's own reporting — to confirm it is genuinely a session
//! leader (`sid == pid`) with a controlling terminal set (`tty_nr != 0`),
//! rather than trusting that a successful `posix_spawn` call means the
//! `POSIX_SPAWN_SETSID` + by-pathname-`addopen` sequence actually did
//! what it's supposed to.

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use platform::process::{Command, GroupSpec};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;
use platform_linux::LinuxPty;

fn size() -> WinSize {
    WinSize { rows: 24, cols: 80 }
}

/// Read `/proc/<pid>/stat`'s session id (field 6) and controlling-terminal
/// device number (field 7, `tty_nr`) — parsed after the `)` that closes
/// the `(comm)` field, since `comm` itself may contain spaces or parens.
fn session_and_tty(pid: u32) -> (i32, i32) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("read /proc/pid/stat");
    let after_comm = stat.rsplit_once(')').expect("stat has a (comm) field").1;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // fields[0] = state, [1] = ppid, [2] = pgrp, [3] = session, [4] = tty_nr
    let session: i32 = fields[3].parse().expect("session field");
    let tty_nr: i32 = fields[4].parse().expect("tty_nr field");
    (session, tty_nr)
}

/// Read from `master` in a loop until `needle` appears in the
/// accumulated output or `attempts` reads have happened — a pty's local
/// echo plus a shell's own output can arrive in more than one `read()`
/// call, and (mirroring the Tun surface's own documented lesson,
/// `docs/convergence-roadmap.md` Phase 8) a freshly-spawned shell may
/// emit its own incidental output first.
fn read_until(master: &dyn PtyMaster, needle: &str, attempts: usize) -> String {
    let mut acc = String::new();
    let mut buf = [0u8; 256];
    for _ in 0..attempts {
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains(needle) {
                    return acc;
                }
            }
            Err(_) => break,
        }
    }
    acc
}

#[test]
fn session_and_controlling_terminal_are_actually_assigned() {
    let pty = LinuxPty;
    let cmd = Command::new("sh", "/").arg("-c").arg("read x");
    let (master, child) = pty.spawn(&cmd, size()).expect("spawn");
    let pid = child.id();

    let (session, tty_nr) = session_and_tty(pid);
    assert_eq!(
        session, pid as i32,
        "child must be its own session leader (sid == pid)"
    );
    assert_ne!(
        tty_nr, 0,
        "child must have a controlling terminal set (tty_nr != 0)"
    );

    // Unblock the child's `read x` so the test doesn't leak a hung
    // process, then reap it.
    master.write(b"\n").expect("write");
    let _ = child.wait();
}

#[test]
fn master_io_round_trips_with_the_spawned_child() {
    let pty = LinuxPty;
    let cmd = Command::new("sh", "/")
        .arg("-c")
        .arg("read line; echo got:$line");
    let (master, child) = pty.spawn(&cmd, size()).expect("spawn");

    master.write(b"hello\n").expect("write");
    // The pty's local echo of the typed line, plus the shell's own
    // "got:hello" output, both arrive on the master.
    let output = read_until(master.as_ref(), "got:hello", 32);
    assert!(
        output.contains("got:hello"),
        "expected 'got:hello' in master output, saw: {output:?}"
    );

    let status = child.wait().expect("wait");
    assert!(status.success());
}

#[test]
fn eof_is_reported_as_ok_zero_after_the_child_exits() {
    let pty = LinuxPty;
    let cmd = Command::new("sh", "/").arg("-c").arg("exit 0");
    let (master, child) = pty.spawn(&cmd, size()).expect("spawn");
    let _ = child.wait();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buf = [0u8; 64];
    loop {
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {
                assert!(Instant::now() < deadline, "never reached EOF within 5s");
            }
            Err(e) => panic!("read must translate EIO to Ok(0), got Err: {e:?}"),
        }
    }
}

#[test]
fn join_group_is_rejected() {
    let pty = LinuxPty;
    let cmd = Command::new("sh", "/").group(GroupSpec::JoinGroup(1));
    let err = pty
        .spawn(&cmd, size())
        .err()
        .expect("JoinGroup must be rejected before attempting a real spawn");
    assert_eq!(err.kind, platform::error::ErrorKind::InvalidInput);
}

#[test]
fn resize_is_visible_to_the_spawned_child() {
    let pty = LinuxPty;
    // Blocks on `read x` until the test writes a line, so the resize
    // below is guaranteed to land before `stty size` runs.
    let cmd = Command::new("sh", "/").arg("-c").arg("read x; stty size");
    let (master, child) = pty.spawn(&cmd, size()).expect("spawn");

    master
        .resize(WinSize {
            rows: 40,
            cols: 120,
        })
        .expect("resize");
    master.write(b"\n").expect("write");

    // `stty size` prints "<rows> <cols>" — kernel ground truth for what
    // the child's own terminal query sees, not this crate's own
    // bookkeeping.
    let output = read_until(master.as_ref(), "40 120", 32);
    assert!(
        output.contains("40 120"),
        "expected stty to report the resized dimensions, saw: {output:?}"
    );
    let _ = child.wait();
}
