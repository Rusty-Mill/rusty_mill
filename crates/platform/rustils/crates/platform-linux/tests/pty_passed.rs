//! Passed descriptors (C75): a pty child gets the descriptors it is
//! passed, in order, at 3, 4, … and nothing else above 2, whatever
//! numbers the parent holds them at. One test, so nothing else in this
//! binary opens or moves descriptors while it puts sources on the target
//! numbers themselves.

#![cfg(target_os = "linux")]
// Putting descriptors on chosen numbers takes raw fcntl and dup2.
#![allow(unsafe_code)]

use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use platform::process::{Child, Command};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use platform_linux::LinuxPty;

/// What the child reports: its own descriptors, then the inode behind
/// each passed one.
const REPORT: &str = "ls /proc/$$/fd | tr '\\n' ' '; echo; \
    for fd in $(seq 3 $((2 + N))); do stat -L -c %i /proc/$$/fd/$fd; done; echo end";

fn run(passed: &[BorrowedFd<'_>]) -> (Vec<i32>, Vec<u64>) {
    let script = REPORT.replace("N", &passed.len().to_string());
    let cmd = Command::new("sh", "/").arg("-c").arg(script);
    let size = WinSize {
        rows: 24,
        cols: 200,
    };
    let (master, child) = LinuxPty.spawn_passing(&cmd, size, passed).expect("spawn");
    let out = read_all(&master);
    Box::new(child).wait().expect("wait");
    let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    let mut fds: Vec<i32> = lines
        .next()
        .expect("fd list")
        .split_whitespace()
        .map(|fd| fd.parse().expect("fd number"))
        .collect();
    fds.sort_unstable();
    let inodes = lines
        .take_while(|l| *l != "end")
        .map(|l| l.parse().expect("inode"))
        .collect();
    (fds, inodes)
}

fn read_all(master: &dyn PtyMaster) -> String {
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    while !acc.contains("end") {
        match master.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => acc.push_str(&String::from_utf8_lossy(&buf[..n])),
        }
    }
    acc
}

fn inode(fd: BorrowedFd<'_>) -> u64 {
    let file = File::from(fd.try_clone_to_owned().expect("dup"));
    file.metadata().expect("fstat").ino()
}

fn temp(name: &str) -> File {
    let dir = std::env::temp_dir().join(format!("pty-passed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    File::create(dir.join(name)).expect("temp file")
}

/// `file` moved to a number at or above 100, freeing its own.
fn lift(file: File) -> OwnedFd {
    // SAFETY: `file` is open; F_DUPFD takes an int.
    let high = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 100) };
    assert!(high >= 100, "F_DUPFD");
    // SAFETY: `high` was just returned and nothing else owns it.
    unsafe { OwnedFd::from_raw_fd(high) }
}

/// `fd` moved to `to`, which must be free, without close-on-exec: the
/// worst case for a stray inheritance.
fn place(fd: OwnedFd, to: i32) -> OwnedFd {
    // SAFETY: fcntl(F_GETFD) on a number we don't own only reads its flags.
    let flags = unsafe { libc::fcntl(to, libc::F_GETFD) };
    assert_eq!(flags, -1, "fd {to} already open");
    // SAFETY: `fd` is open; `to` is free, so dup2 replaces nothing.
    assert_eq!(unsafe { libc::dup2(fd.as_raw_fd(), to) }, to, "dup2");
    // SAFETY: `to` was just opened by dup2 and nothing else owns it.
    unsafe { OwnedFd::from_raw_fd(to) }
}

#[test]
fn passed_descriptors_land_in_order_and_nothing_else_leaks() {
    // None passed: a plain spawn, unchanged.
    let (fds, _) = run(&[]);
    assert_eq!(fds, [0, 1, 2], "no descriptor above 2");

    // Overlap: the sources sit on each other's targets, swapped. Sparse:
    // unrelated inheritable descriptors at 5 and 9 must not reach the child.
    let [first, second, five, nine] = ["first", "second", "five", "nine"].map(|n| lift(temp(n)));
    let first = place(first, 4);
    let second = place(second, 3);
    let stray = [place(five, 5), place(nine, 9)];
    let passed = [first.as_fd(), second.as_fd()];
    let (fds, inodes) = run(&passed);
    assert_eq!(fds, [0, 1, 2, 3, 4]);
    assert_eq!(
        inodes,
        [inode(first.as_fd()), inode(second.as_fd())],
        "launch order"
    );
    drop(stray);

    // The most a launch passes (C75: 8), from ordinary numbers.
    let files: Vec<File> = (0..8).map(|i| temp(&format!("f{i}"))).collect();
    let passed: Vec<BorrowedFd<'_>> = files.iter().map(AsFd::as_fd).collect();
    let (fds, inodes) = run(&passed);
    assert_eq!(fds, (0..11).collect::<Vec<_>>());
    let want: Vec<u64> = files
        .iter()
        .map(|f| f.metadata().expect("fstat").ino())
        .collect();
    assert_eq!(inodes, want);
}
