//! A closed stdout must not panic a lens subcommand: the report would be
//! lost with it. After the first smoke run (2026-11-17) the terminal closed
//! while `lens run` was working, `print!` panicked with "Broken pipe" and the
//! whole run record vanished. The text now goes to stderr and the exit code
//! stays that of the work.
use std::process::{Command, Stdio};

/// Run `rusty_orch` with its stdout pipe's read end already closed.
fn run_with_closed_stdout(args: &[&str]) -> (Option<i32>, String) {
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    let out = Command::new(env!("CARGO_BIN_EXE_rusty_orch"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()
        .expect("run rusty_orch");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn help_with_a_closed_stdout_does_not_panic_and_reaches_stderr() {
    let (code, stderr) = run_with_closed_stdout(&["lens", "check", "--help"]);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(stderr.contains("usage: rusty_orch lens check"), "{stderr}");
    assert_eq!(code, Some(0), "{stderr}");
}

#[test]
fn a_report_with_a_closed_stdout_does_not_panic_and_reaches_stderr() {
    let config = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../orch-lens/tests/fixtures/lens/lens.json"
    );
    let (code, stderr) = run_with_closed_stdout(&[
        "lens",
        "check",
        config,
        "--assertions",
        "/nonexistent/assertions.json",
        "--repo",
        env!("CARGO_MANIFEST_DIR"),
    ]);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(
        stderr.contains("claude\tclaude-cli\tunavailable"),
        "{stderr}"
    );
    assert!(stderr.contains("local\topenai-http"), "{stderr}");
    assert_eq!(code, Some(0), "{stderr}");
}
