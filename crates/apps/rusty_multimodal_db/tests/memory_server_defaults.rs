//! `ADR-0099` (`DEF-FR-001`–`DEF-FR-003`) against the compiled
//! `memory_server`: with nothing set, the idle timeout, the connection
//! cap, the row cap (`ADR-0102`), and synced updates are on at their
//! defaults; `0` turns each off; a malformed value refuses to start. The
//! ready banner on stderr is the observable — it prints the effective
//! values — since a 300 s timeout and a 1,024-connection cap are not
//! things a test can afford to trip.

#![cfg(unix)]

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
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

fn memory_server(env: &[(&str, &str)]) -> (Command, SocketAddr) {
    let addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    command
        .arg(addr.to_string())
        .env("SERVER_DATA_DIR", unique_dir("memory_server_defaults"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, v) in env {
        command.env(k, v);
    }
    (command, addr)
}

/// Start, wait for the listener, kill, and return everything the
/// server wrote to stderr (the ready banner is the last line).
fn banner(env: &[(&str, &str)]) -> String {
    let (mut command, addr) = memory_server(env);
    let mut child: Child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(addr).is_err() {
        assert!(
            Instant::now() < deadline,
            "memory_server never listened on {addr}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn with_nothing_set_the_limits_and_synced_updates_are_on() {
    let text = banner(&[]);
    assert!(text.contains("idle timeout: Some(300s)"), "banner: {text}");
    assert!(
        text.contains("max connections: Some(1024)"),
        "banner: {text}"
    );
    assert!(
        text.contains("max query rows: Some(10000)"),
        "banner: {text}"
    );
    assert!(
        text.contains("synced updates: configured"),
        "banner: {text}"
    );
}

#[test]
fn zero_turns_each_default_off_and_a_positive_value_sets_it() {
    let text = banner(&[
        ("SERVER_IDLE_TIMEOUT_SECS", "0"),
        ("SERVER_MAX_CONNECTIONS", "7"),
        ("SERVER_MAX_QUERY_ROWS", "50"),
        ("SERVER_SYNC_UPDATES", "0"),
    ]);
    assert!(text.contains("idle timeout: None"), "banner: {text}");
    assert!(text.contains("max connections: Some(7)"), "banner: {text}");
    assert!(text.contains("max query rows: Some(50)"), "banner: {text}");
    assert!(
        text.contains("synced updates: NOT configured"),
        "banner: {text}"
    );
}

#[test]
fn a_malformed_setting_is_a_startup_error_that_names_it() {
    for (name, value) in [
        ("SERVER_IDLE_TIMEOUT_SECS", "soon"),
        ("SERVER_MAX_CONNECTIONS", "-1"),
        ("SERVER_SYNC_UPDATES", "yes"),
    ] {
        let (mut command, addr) = memory_server(&[(name, value)]);
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{name}={value} started; stderr: {stderr}"
        );
        assert!(
            stderr.contains(name),
            "the refusal did not name {name}: {stderr}"
        );
        assert!(TcpStream::connect(addr).is_err());
    }
}
