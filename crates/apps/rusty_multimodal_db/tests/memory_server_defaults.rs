//! `ADR-0099` (`DEF-FR-001`–`DEF-FR-003`) against the compiled
//! `memory_server`: with nothing set, the idle timeout, the connection
//! cap, the row cap (`ADR-0102`), and synced updates are on at their
//! defaults; `0` turns each off; a malformed value refuses to start. The
//! ready banner on stderr is the observable — it prints the effective
//! values — since a 300 s timeout and a 1,024-connection cap are not
//! things a test can afford to trip.

#![cfg(unix)]

use std::net::SocketAddr;
use std::path::PathBuf;
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
    #[allow(dead_code)]
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

fn memory_server(env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    command
        .arg("127.0.0.1:0")
        .env("SERVER_DATA_DIR", unique_dir("memory_server_defaults"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, v) in env {
        command.env(k, v);
    }
    command
}

/// Start, read stderr up to and including the ready banner, kill, and
/// return it (the banner is the last line).
fn banner(env: &[(&str, &str)]) -> String {
    let server = spawn_listening(memory_server(env));
    format!("{}{}\n", server.before_banner, server.banner)
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
    assert!(
        text.contains("MVCC reclaim every: Some(10000)"),
        "banner: {text}"
    );
    assert!(
        text.contains("journaled updates: NOT configured"),
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
        ("SERVER_MVCC_RECLAIM_EVERY", "0"),
    ]);
    assert!(text.contains("MVCC reclaim every: None"), "banner: {text}");
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
        let mut command = memory_server(&[(name, value)]);
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
    }
}
