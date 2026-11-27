//! The binary end to end: MCP over stdio, one critic and one builder session.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
const READY: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

fn root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orch-fs-shell-e2e-{}-{name}", std::process::id()));
    let _stale = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("root");
    dir
}

/// Send `requests` after the handshake and answer everything the server wrote.
fn session(root: &Path, mode: &str, requests: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_orch-fs-shell"))
        .args(["--root", root.to_str().expect("utf-8 root"), "--mode", mode])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().expect("stdin");
    for line in [INIT, READY].iter().chain(requests) {
        writeln!(stdin, "{line}").expect("write");
    }
    drop(stdin);
    let out = child.wait_with_output().expect("output");
    String::from_utf8(out.stdout).expect("utf-8 stdout")
}

#[test]
fn critic_reads_but_is_not_offered_write() {
    let dir = root("critic");
    std::fs::write(dir.join("a.txt"), "hello").expect("seed");
    let out = session(
        &dir,
        "critic",
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read","arguments":{"path":"a.txt"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"read","arguments":{"path":"../../etc/passwd"}}}"#,
        ],
    );
    assert!(out.contains(r#""name":"exec""#), "{out}");
    assert!(!out.contains(r#""name":"write""#), "{out}");
    assert!(out.contains(r#"\"content\":\"hello\""#), "{out}");
    assert!(out.contains("is refused"), "{out}");
    assert!(!out.contains("root:"), "no passwd line leaked: {out}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builder_writes_and_runs() {
    let dir = root("builder");
    // Calls may run concurrently, so exec reads a seeded file, not the write.
    std::fs::write(dir.join("seed.txt"), "seeded").expect("seed");
    let out = session(
        &dir,
        "builder",
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"write","arguments":{"path":"n.txt","content":"made"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"exec","arguments":{"argv":["cat","seed.txt"]}}}"#,
        ],
    );
    assert!(out.contains(r#"\"written\":4"#), "{out}");
    assert!(out.contains(r#"\"stdout\":\"seeded\""#), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.join("n.txt")).expect("file"),
        "made"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_flags_exit_with_usage() {
    let out = Command::new(env!("CARGO_BIN_EXE_orch-fs-shell"))
        .args(["--mode", "critic"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage:"));
}
