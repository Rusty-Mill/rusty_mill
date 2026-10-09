//! End-to-end regression for the MCP rule that stdout is protocol-only.

use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn verbose_stdio_keeps_diagnostics_off_stdout() {
    let forge = tempfile::tempdir().unwrap();
    nexus_bootstrap::init_forge(forge.path()).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["-v", "--forge-path"])
        .arg(forge.path())
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "stdout-regression", "version": "1" }
        }
    });
    writeln!(child.stdin.take().unwrap(), "{request}").unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let messages: Vec<serde_json::Value> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|error| {
                panic!("non-protocol stdout ({error}): {line:?}; full stdout: {stdout:?}")
            })
        })
        .collect();
    assert_eq!(messages.len(), 1, "stdout: {stdout:?}");
    assert_eq!(messages[0]["id"], 1);

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("serving MCP over stdio"),
        "expected verbose diagnostics on stderr, got: {stderr:?}"
    );
}
