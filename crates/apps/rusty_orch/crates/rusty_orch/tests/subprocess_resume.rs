//! A real binary/process-boundary persistence smoke test. The model CLI is a
//! local script, so this tests process reopen without requiring a login.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rusty_orch_subprocess_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run(
    binary: &str,
    goal: &std::path::Path,
    state: &std::path::Path,
    bin: &std::path::Path,
    interactive: bool,
) -> std::process::Output {
    let mut command = Command::new(binary);
    command
        .arg("run")
        .arg(goal)
        .arg("--state")
        .arg(state)
        .arg("--json")
        .env("PATH", bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if interactive {
        command.arg("--interactive");
    }
    let mut child = command.spawn().expect("spawn binary");
    if interactive {
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(b"continue\n")
            .expect("answer");
    }
    child.wait_with_output().expect("wait")
}

#[test]
fn built_binary_reopens_state_in_a_second_os_process() {
    let root = temp_dir();
    let bin = root.join("bin");
    let state = root.join("state");
    std::fs::create_dir(&bin).expect("bin dir");
    let marker = root.join("called");
    let ollama = bin.join("ollama");
    std::fs::write(
        &ollama,
        format!(
            "#!/bin/sh\ncat >/dev/null\nif [ -e '{}' ]; then\n  printf '%s\\n' '{{\"entries\":[{{\"kind\":\"finding\",\"confidence\":\"high\",\"body\":\"done\",\"refs\":[]}}]}}'\nelse\n  : > '{}'\n  printf '%s\\n' '{{\"entries\":[{{\"kind\":\"question\",\"body\":\"continue?\",\"refs\":[]}}]}}'\nfi\n",
            marker.display(), marker.display()
        ),
    )
    .expect("script");
    let mut permissions = std::fs::metadata(&ollama).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&ollama, permissions).expect("chmod");

    let goal = root.join("goal.json");
    std::fs::write(&goal, r#"{"goal":"g","done_when":["done"],"out_of_scope":[],"wall_clock_secs":30,"max_calls":3,"stop":"checkpoint","routing":{"research":"local"},"tasks":[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":3}]}"#).expect("goal");
    let binary = env!("CARGO_BIN_EXE_rusty_orch");

    let first = run(binary, &goal, &state, &bin, false);
    assert_eq!(
        first.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("\"blocked\""));

    let second = run(binary, &goal, &state, &bin, true);
    assert_eq!(
        second.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("\"finished\""), "{stdout}");
    assert!(stdout.contains("\"calls\": 2"), "{stdout}");
    let _ = std::fs::remove_dir_all(root);
}
