//! `DRN-FR-005` (ADR-0127): `memory_server` drains on SIGTERM and exits 0.
//! Linux only — elsewhere the binary installs no handler and runs until
//! killed, as it did before.
#![cfg(target_os = "linux")]

use std::io::BufRead;
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn spawn(data_dir: &std::path::Path) -> (std::process::Child, SocketAddr) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    let mut child = command
        .arg("127.0.0.1:0")
        .env("SERVER_DATA_DIR", data_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    for line in std::io::BufReader::new(stderr).lines() {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("memory_server listening on ") {
            let addr = rest.split(' ').next().unwrap().parse().unwrap();
            // Keep draining stderr so the child never blocks on a full pipe.
            return (child, addr);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("stderr closed before the listening banner");
}

fn signal(child: &std::process::Child, name: &str) {
    let status = Command::new("kill")
        .arg(format!("-{name}"))
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());
}

fn wait_within(child: &mut std::process::Child, limit: Duration) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if started.elapsed() > limit {
            child.kill().unwrap();
            panic!("memory_server did not exit within {limit:?} of the signal");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn sigterm_drains_an_idle_server_and_it_exits_cleanly() {
    let dir = std::env::temp_dir().join(format!("drain_bin_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut child, addr) = spawn(&dir);
    let connection = TcpStream::connect(addr).unwrap(); // an idle client
    signal(&child, "TERM");
    let status = wait_within(&mut child, Duration::from_secs(10));
    assert!(status.success(), "a drained server exits 0, got {status:?}");
    drop(connection);
    assert!(TcpStream::connect(addr).is_err(), "the listener is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sigint_drains_the_same_way() {
    let dir = std::env::temp_dir().join(format!("drain_bin_int_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut child, _addr) = spawn(&dir);
    signal(&child, "INT");
    assert!(wait_within(&mut child, Duration::from_secs(10)).success());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Start `memory_server` on `data` with a change log in `logs`; return the
/// child and the `(epoch, head)` it reports for the `memory` table.
fn spawn_with_log(
    data: &std::path::Path,
    logs: &std::path::Path,
) -> (std::process::Child, SocketAddr, (u64, u64)) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    let mut child = command
        .arg("127.0.0.1:0")
        .env("SERVER_DATA_DIR", data)
        .env("SERVER_CHANGE_LOG_DIR", logs)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut addr = None;
    let mut lines = std::io::BufReader::new(stderr).lines();
    while let Some(line) = lines.next() {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("memory_server listening on ") {
            addr = Some(rest.split(' ').next().unwrap().parse().unwrap());
        }
        if let Some(rest) = line.strip_prefix("memory_server change log for memory: ") {
            let inner = rest.rsplit_once('(').unwrap().1.trim_end_matches(')');
            let num = |key: &str| -> u64 {
                inner
                    .split(", ")
                    .find_map(|kv| kv.strip_prefix(key))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap()
            };
            let position = (num("epoch "), num("head "));
            // Keep draining stderr: the rest of the startup lines must not
            // hit a closed pipe.
            std::thread::spawn(move || for _ in lines {});
            return (child, addr.expect("the banner came first"), position);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("stderr closed before the change log line");
}

/// `CHL-FR-003`/`006` (ADR-0131): a drained `memory_server` continues the
/// change log's epoch at the next start; one that was killed starts a new one.
#[test]
fn a_clean_drain_keeps_the_epoch_and_a_kill_starts_a_new_one() {
    let root = std::env::temp_dir().join(format!("drain_log_{}", std::process::id()));
    let (data, logs) = (root.join("data"), root.join("logs"));
    std::fs::create_dir_all(&data).unwrap();

    let (mut child, _, first) = spawn_with_log(&data, &logs);
    assert_eq!(first, (1, 0), "a fresh log: epoch 1, nothing recorded");
    signal(&child, "TERM");
    assert!(wait_within(&mut child, Duration::from_secs(10)).success());

    let (mut child, _, second) = spawn_with_log(&data, &logs);
    assert_eq!(second, (1, 0), "said goodbye, so the epoch continues");
    child.kill().unwrap();
    let _ = child.wait();

    let (mut child, _, third) = spawn_with_log(&data, &logs);
    assert_eq!(third.0, 2, "killed, so a new epoch");
    signal(&child, "TERM");
    assert!(wait_within(&mut child, Duration::from_secs(10)).success());
    let _ = std::fs::remove_dir_all(&root);
}
