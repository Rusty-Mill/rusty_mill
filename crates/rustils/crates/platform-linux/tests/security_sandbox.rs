//! Sandbox integration test (RFC v2 R5+, D15, Phase 6 item 3).
//!
//! `confine_filesystem`/`block_inet_sockets` are irreversible for the
//! calling thread, and `cargo test`'s default harness reuses a thread
//! pool across tests — confining a thread in-process here could silently
//! break a *different*, later test scheduled onto the same reused
//! thread. Each check below re-execs the test binary into a fresh,
//! single-test child process instead, mirroring (at test-infrastructure
//! scale) the same "irreversible confinement belongs in its own process"
//! shape nexus's own helper-binary design settled on for the real thing.

#![cfg(target_os = "linux")]

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use platform::security::{Sandbox, SandboxStatus};

const REEXEC_ENV: &str = "RUSTILS_SANDBOX_TEST_CHILD";

#[test]
fn confine_filesystem_enforces_read_write_boundaries() {
    if env::var(REEXEC_ENV).as_deref() == Ok("filesystem") {
        run_filesystem_child();
        return;
    }
    let status = reexec(
        "confine_filesystem_enforces_read_write_boundaries",
        "filesystem",
    );
    assert!(status.success(), "child exited with {status:?}");
}

#[test]
fn block_inet_sockets_blocks_inet_allows_unix() {
    if env::var(REEXEC_ENV).as_deref() == Ok("network") {
        run_network_child();
        return;
    }
    let status = reexec("block_inet_sockets_blocks_inet_allows_unix", "network");
    assert!(status.success(), "child exited with {status:?}");
}

fn reexec(test_name: &str, mode: &str) -> std::process::ExitStatus {
    let exe = env::current_exe().expect("current_exe");
    Command::new(exe)
        .arg(test_name)
        .arg("--exact")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(REEXEC_ENV, mode)
        .status()
        .expect("spawn re-exec child")
}

fn run_filesystem_child() {
    let base = env::temp_dir().join(format!("rustils-sandbox-fs-{}", std::process::id()));
    let readable = base.join("readable");
    let writable = base.join("writable");
    let excluded = base.join("excluded");
    fs::create_dir_all(&readable).unwrap();
    fs::create_dir_all(&writable).unwrap();
    fs::create_dir_all(&excluded).unwrap();
    fs::write(readable.join("r.txt"), b"readable content").unwrap();
    fs::write(excluded.join("e.txt"), b"excluded content").unwrap();

    let sandbox = platform_linux::LinuxSandbox;
    let readable_root: &Path = &readable;
    let writable_root: &Path = &writable;
    let status = sandbox
        .confine_filesystem(&[readable_root], &[writable_root])
        .unwrap();
    if status == SandboxStatus::NotEnforced {
        // Kernel lacks Landlock (missing entirely, pre-5.13, or disabled) —
        // the degrade path itself is exercised, but not real enforcement.
        eprintln!("Landlock unavailable in this environment; degrade path only");
        return;
    }
    assert_eq!(status, SandboxStatus::Enforced);

    assert!(
        fs::read(readable.join("r.txt")).is_ok(),
        "readable root: read"
    );
    assert!(
        fs::write(readable.join("new.txt"), b"x").is_err(),
        "readable root must not be writable"
    );
    assert!(
        fs::write(writable.join("ok.txt"), b"x").is_ok(),
        "writable root: write"
    );
    assert!(
        fs::read(writable.join("ok.txt")).is_ok(),
        "writable root: read back"
    );
    assert!(
        fs::read(excluded.join("e.txt")).is_err(),
        "excluded root must not be reachable at all"
    );
}

fn run_network_child() {
    use std::net::{TcpListener, UdpSocket};

    let base = env::temp_dir().join(format!("rustils-sandbox-net-{}", std::process::id()));
    fs::create_dir_all(&base).unwrap();
    assert!(
        std::os::unix::net::UnixListener::bind(base.join("pre.sock")).is_ok(),
        "AF_UNIX must work before confinement"
    );

    let sandbox = platform_linux::LinuxSandbox;
    let status = sandbox.block_inet_sockets().unwrap();
    assert_eq!(
        status,
        SandboxStatus::Enforced,
        "seccomp filter mode has existed since Linux 3.5 — expected on any test host"
    );

    assert!(
        TcpListener::bind("127.0.0.1:0").is_err(),
        "AF_INET must be blocked after confinement"
    );
    assert!(
        UdpSocket::bind("127.0.0.1:0").is_err(),
        "AF_INET (UDP) must be blocked after confinement"
    );
    assert!(
        std::os::unix::net::UnixListener::bind(base.join("post.sock")).is_ok(),
        "AF_UNIX must remain unaffected"
    );
}
