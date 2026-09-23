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
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

/// `RGT-FR-004` (ADR-0123): a server spawned on port 0, its bound address
/// read from the listening banner — no port is picked ahead of the bind,
/// so two tests can never race for one. `before_banner` is every stderr
/// line the server wrote before it; the pipe stays open for its lifetime.
struct Server {
    child: Child,
    addr: SocketAddr,
    // The same helper in every binary test; each reads the fields it needs.
    #[allow(dead_code)]
    before_banner: String,
    /// The listening banner itself, the server's own account of its settings.
    #[allow(dead_code)]
    banner: String,
    _stderr: std::io::Lines<std::io::BufReader<std::process::ChildStderr>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_listening(mut command: Command) -> Server {
    use std::io::BufRead;
    let mut child = command.stderr(Stdio::piped()).spawn().unwrap();
    let stderr = child.stderr.take().expect("stderr is piped");
    let mut lines = std::io::BufReader::new(stderr).lines();
    let mut before_banner = String::new();
    let (addr, banner) = loop {
        let line = lines
            .next()
            .unwrap_or_else(|| panic!("stderr closed before the listening banner: {before_banner}"))
            .unwrap();
        if let Some(rest) = line.strip_prefix("memory_server listening on ") {
            let addr = rest
                .split(' ')
                .next()
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap();
            break (addr, line);
        }
        before_banner.push_str(&line);
        before_banner.push('\n');
    };
    Server {
        child,
        addr,
        before_banner,
        banner,
        _stderr: lines,
    }
}

fn memory_server_command(data_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    command
        .arg("127.0.0.1:0")
        .env("SERVER_DATA_DIR", data_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

/// `DDL-FR-002`: the first server claims the directory and the lock
/// file appears; `DDL-FR-003`: a second server on the same directory
/// exits non-zero before listening, naming the lock file on stderr; once
/// the first is gone the directory can be claimed again — no stale
/// lock to clear by hand.
#[test]
fn a_second_memory_server_on_the_same_data_dir_refuses_to_start_until_the_first_exits() {
    let data_dir = unique_dir("memory_server_data_dir_lock");
    let first = spawn_listening(memory_server_command(&data_dir));
    assert!(
        data_dir.join(LOCK_FILE_NAME).is_file(),
        "the running server left no lock file in {data_dir:?}"
    );

    let second = memory_server_command(&data_dir).output().unwrap();
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

    drop(first);
    let third = spawn_listening(memory_server_command(&data_dir));
    assert!(TcpStream::connect(third.addr).is_ok());
    drop(third);
    let _ = std::fs::remove_dir_all(&data_dir);
}
