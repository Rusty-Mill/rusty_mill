//! `memory_server` takes one process per `SERVER_DATA_DIR` (`ADR-0092`,
//! `DDL-FR-002`/`DDL-FR-003`) — proven against the real compiled binary,
//! spawned as a subprocess (`env!("CARGO_BIN_EXE_memory_server")`,
//! mirroring `tests/memory_server_mvcc_integration.rs`), because the
//! lock is an OS-level `flock` on an open file description: only a
//! second *process* exercises what `src/server/data_lock.rs`'s own unit
//! tests cannot (an in-process second claim proves the same syscall, not
//! the deployment wiring in `main`).

#![cfg(unix)]

use rusty_multimodal_db::server::data_lock::LOCK_FILE_NAME;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn memory_server_command(data_dir: &Path, addr: SocketAddr) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    command
        .arg(addr.to_string())
        .env("SERVER_DATA_DIR", data_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn wait_for_listener(addr: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("memory_server never started listening on {addr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `DDL-FR-002`: the first server claims the directory and the lock
/// file appears; `DDL-FR-003`: a second server on the same directory
/// exits non-zero before listening, naming the lock file on stderr; once
/// the first is gone the directory can be claimed again — no stale
/// lock to clear by hand.
#[test]
fn a_second_memory_server_on_the_same_data_dir_refuses_to_start_until_the_first_exits() {
    let data_dir = unique_dir("memory_server_data_dir_lock");
    let first_addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let first = ChildGuard(
        memory_server_command(&data_dir, first_addr)
            .spawn()
            .unwrap(),
    );
    wait_for_listener(first_addr);
    assert!(
        data_dir.join(LOCK_FILE_NAME).is_file(),
        "the running server left no lock file in {data_dir:?}"
    );

    let second_addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let second = memory_server_command(&data_dir, second_addr)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        !second.status.success(),
        "a second server on {data_dir:?} started anyway; stderr: {stderr}"
    );
    assert!(
        stderr.contains("held by another process") && stderr.contains(LOCK_FILE_NAME),
        "the refusal did not name the holder's lock file: {stderr}"
    );
    assert!(
        !stderr.contains("listening on"),
        "the refused server printed a listening banner before refusing: {stderr}"
    );
    assert!(
        TcpStream::connect(second_addr).is_err(),
        "the refused server is listening on {second_addr}"
    );

    drop(first);
    let third_addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let third = ChildGuard(
        memory_server_command(&data_dir, third_addr)
            .spawn()
            .unwrap(),
    );
    wait_for_listener(third_addr);
    drop(third);
    let _ = std::fs::remove_dir_all(&data_dir);
}
