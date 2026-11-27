//! Passed descriptors through `LinuxSpawner::spawn_passing`, and copying a
//! child's descriptor back with `LinuxChild::dup_fd` (#1318 S1: orch places
//! the builder's slot at fd 3 and its ACP end at fd 4, and takes the
//! holder's slot descriptor with `pidfd_getfd`).
//!
//! One test, so nothing else in this binary opens or moves descriptors
//! while it puts sources on the target numbers themselves.

#![cfg(target_os = "linux")]
// Placing descriptors on chosen numbers and flock take raw libc calls.
#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use platform::error::{ErrorKind, OsCode};
use platform::process::{Child, Command, Signal, Stdio};
use platform_linux::LinuxSpawner;

/// What the child reports: its own descriptors, then the inode behind each
/// passed one.
const REPORT: &str = "ls /proc/$$/fd | tr '\\n' ' '; echo; \
    for fd in $(seq 3 $((2 + N))); do stat -L -c %i /proc/$$/fd/$fd; done";

fn run(passed: &[BorrowedFd<'_>]) -> (Vec<i32>, Vec<u64>) {
    let script = REPORT.replace("N", &passed.len().to_string());
    let mut cmd = Command::new("sh", "/").arg("-c").arg(script);
    cmd.stdout = Stdio::Pipe;
    let mut child = LinuxSpawner.spawn_passing(&cmd, passed).expect("spawn");
    let mut out = child.take_stdout().expect("stdout pipe");
    let mut text = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match out.read(&mut buf).expect("read") {
            0 => break,
            n => text.push_str(&String::from_utf8_lossy(&buf[..n])),
        }
    }
    let status = Box::new(child).wait().expect("wait");
    assert!(status.success(), "report script failed: {text}");
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let mut fds: Vec<i32> = lines
        .next()
        .expect("fd list")
        .split_whitespace()
        .map(|fd| fd.parse().expect("fd number"))
        .collect();
    fds.sort_unstable();
    let inodes = lines.map(|l| l.parse().expect("inode")).collect();
    (fds, inodes)
}

fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("spawn-passed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn temp(name: &str) -> File {
    File::create(dir().join(name)).expect("temp file")
}

fn inode(fd: BorrowedFd<'_>) -> u64 {
    let file = File::from(fd.try_clone_to_owned().expect("dup"));
    file.metadata().expect("fstat").ino()
}

/// `fd` moved to `to`, which must be free, without close-on-exec: the worst
/// case for a stray inheritance.
fn place(fd: OwnedFd, to: i32) -> OwnedFd {
    // SAFETY: fcntl(F_GETFD) on a number we don't own only reads its flags.
    let flags = unsafe { libc::fcntl(to, libc::F_GETFD) };
    assert_eq!(flags, -1, "fd {to} already open");
    // SAFETY: `fd` is open; `to` is free, so dup2 replaces nothing.
    assert_eq!(unsafe { libc::dup2(fd.as_raw_fd(), to) }, to, "dup2");
    // SAFETY: `to` was just opened by dup2 and nothing else owns it.
    unsafe { OwnedFd::from_raw_fd(to) }
}

/// `file` moved to a number at or above 100, freeing its own.
fn lift(file: File) -> OwnedFd {
    // SAFETY: `file` is open; F_DUPFD takes an int.
    let high = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 100) };
    assert!(high >= 100, "F_DUPFD");
    // SAFETY: `high` was just returned and nothing else owns it.
    unsafe { OwnedFd::from_raw_fd(high) }
}

/// `flock(LOCK_EX | LOCK_NB)`: true when taken.
fn try_lock(fd: BorrowedFd<'_>) -> bool {
    // SAFETY: `fd` is open; flock takes plain integers.
    unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// A child that sleeps with `passed` at 3, 4, ….
fn sleeper(passed: &[BorrowedFd<'_>]) -> platform_linux::LinuxChild {
    let cmd = Command::new("sleep", "/").arg("30");
    LinuxSpawner
        .spawn_passing(&cmd, passed)
        .expect("spawn sleeper")
}

fn passed_descriptors_land_in_order_and_nothing_else_leaks() {
    // Overlap: the sources sit on each other's targets, swapped. Sparse:
    // unrelated inheritable descriptors at 5 and 9 must not reach the child.
    let [first, second, five, nine] = ["first", "second", "five", "nine"].map(|n| lift(temp(n)));
    let first = place(first, 4);
    let second = place(second, 3);
    let stray = [place(five, 5), place(nine, 9)];
    let passed = [first.as_fd(), second.as_fd()];
    let (fds, inodes) = run(&passed);
    // 0–2 are stdio (1 the pipe); 3 and 4 the passed ones; nothing else.
    assert_eq!(fds, [0, 1, 2, 3, 4]);
    assert_eq!(
        inodes,
        [inode(first.as_fd()), inode(second.as_fd())],
        "launch order"
    );
    drop(stray);
}

fn a_copied_descriptor_shares_the_childs_lock() {
    let path = dir().join("slot");
    let slot = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .expect("slot file");
    assert!(try_lock(slot.as_fd()), "lock the slot");
    let child = sleeper(&[slot.as_fd()]);
    drop(slot);

    let copy = child.dup_fd(3).expect("dup_fd");
    // Same open file description: the lock is already ours on the copy…
    assert!(try_lock(copy.as_fd()), "the copy holds the lock");
    // …and a fresh open of the same file is refused it.
    let other = File::open(&path).expect("reopen");
    assert!(!try_lock(other.as_fd()), "a second flock fails");

    // The lock outlives the child while the copy is open.
    child.kill_single(Signal::Kill).expect("kill");
    Box::new(child).wait().expect("wait");
    assert!(!try_lock(other.as_fd()), "still held by the copy");
    drop(copy);
    assert!(try_lock(other.as_fd()), "freed with the last copy");
}

fn refused_copies_say_why() {
    let child = sleeper(&[]);
    let err = child.dup_fd(1_000_000).expect_err("no such descriptor");
    assert_eq!(err.os, OsCode::Errno(libc::EBADF));
    child.kill_single(Signal::Kill).expect("kill");

    let mut child = child;
    while child.try_wait().expect("try_wait").is_none() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let err = child.dup_fd(0).expect_err("reaped");
    assert_eq!(err.kind, ErrorKind::InvalidInput);
}

#[test]
fn spawn_passing_and_dup_fd() {
    passed_descriptors_land_in_order_and_nothing_else_leaks();
    a_copied_descriptor_shares_the_childs_lock();
    refused_copies_say_why();
}
