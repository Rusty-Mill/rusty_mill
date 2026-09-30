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
