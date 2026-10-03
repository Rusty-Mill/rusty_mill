//! The entry point over captured streams: stdout must stay a single JSON
//! object even when the run asks and takes an answer.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Duration;

use orch_cli::fake::{exit, FakeCommand};
use orch_core::board::{Confidence, EntryKind};
use orch_dispatch::fake::{FakeAgent, Reply};
use orch_ollama::OllamaAgent;
use rusty_json::Value;
use rusty_orch::args::Args;
use rusty_orch::cli::{self, Streams, EXIT_BLOCKED, EXIT_FAILED, EXIT_FINISHED};

const GOAL: &str = r#"{"goal":"g","done_when":["d"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":5,"stop":"checkpoint",
  "tasks":[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":2}]}"#;

fn args(interactive: bool, json: bool) -> Args {
    Args {
        goal: PathBuf::from("goal.json"),
        ollama_model: "m".into(),
        codex_repo: PathBuf::from("."),
        codex_model: None,
        interactive,
        json,
    }
}

fn run(args: &Args, runner: FakeAgent, stdin: &str) -> (u8, String, String) {
    let mut stdin = Cursor::new(stdin.as_bytes().to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = {
        let mut streams = Streams {
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut stderr,
        };
        cli::run(args, GOAL, runner, &mut streams).expect("run")
    };
    (
        code,
        String::from_utf8(stdout).expect("utf8"),
        String::from_utf8(stderr).expect("utf8"),
    )
}

#[test]
fn interactive_json_keeps_stdout_to_one_parseable_object() {
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![EntryKind::Finding {
            confidence: Confidence::High,
        }]),
    ]);
    let (code, stdout, stderr) = run(&args(true, true), fake, "yes, proceed\n");

    assert_eq!(code, EXIT_FINISHED);
    let json = Value::parse(&stdout).expect("stdout is exactly one JSON document");
    assert_eq!(
        json.get("ended")
            .and_then(|e| e.get("kind"))
            .and_then(|k| k.as_str()),
        Some("finished")
    );
    assert_eq!(json.get("calls").and_then(|c| c.as_u64()), Some(2));
    assert!(
        stderr.contains("E-1: fake Question"),
        "the question went to stderr:\n{stderr}"
    );
    assert!(stderr.contains("answer (blank to stop)>"));
    assert!(stderr.contains("run: blocked on 1 card(s)"));
    assert!(
        !stdout.contains("answer (blank to stop)"),
        "stdout is clean:\n{stdout}"
    );
}

#[test]
fn a_blank_line_on_stdin_stops_the_run_blocked() {
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![EntryKind::Finding {
            confidence: Confidence::High,
        }]),
    ]);
    let (code, stdout, _) = run(&args(true, true), fake, "\n");

    assert_eq!(code, EXIT_BLOCKED);
    let json = Value::parse(&stdout).expect("one JSON document");
    assert_eq!(
        json.get("ended")
            .and_then(|e| e.get("kind"))
            .and_then(|k| k.as_str()),
        Some("blocked")
    );
    assert_eq!(json.get("calls").and_then(|c| c.as_u64()), Some(1));
}

#[test]
fn non_interactive_never_reads_stdin_and_prints_text() {
    let fake = FakeAgent::new([Reply::Write(vec![EntryKind::Question])]);
    let (code, stdout, stderr) = run(&args(false, false), fake, "an answer nobody asked for\n");

    assert_eq!(code, EXIT_BLOCKED);
    assert!(stdout.contains("ended: blocked on 1 card(s)"), "{stdout}");
    assert!(
        !stderr.contains("answer (blank to stop)"),
        "nothing was asked:\n{stderr}"
    );
}

/// Harmless marker standing in for anything a model or a child process might
/// say. It must reach the report and must never reach a progress line.
const SENTINEL: &str = "SENTINEL-7c1e-must-not-leak";

/// Every stderr line that is progress, i.e. not a question or its prompt.
fn progress_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|l| {
            l.starts_with("run: ")
                || l.contains(": blank answer ignored")
                || l.contains(": answer refused")
        })
        .collect()
}

#[test]
fn a_synthetic_agent_error_reaches_the_report_but_not_progress() {
    let fake = FakeAgent::new([Reply::Fail(format!("boom {SENTINEL}"))]);
    let (code, stdout, stderr) = run(&args(false, true), fake, "");

    assert_eq!(code, EXIT_FAILED);
    let json = Value::parse(&stdout).expect("one JSON document");
    let error = json
        .get("ended")
        .and_then(|e| e.get("error"))
        .and_then(|k| k.as_str())
        .expect("error");
    assert!(
        error.contains(SENTINEL),
        "the report keeps the detail: {error}"
    );
    assert!(
        !stderr.contains(SENTINEL),
        "progress must not carry it:\n{stderr}"
    );
    let progress = progress_lines(&stderr);
    assert_eq!(progress.len(), 1, "{stderr}");
    assert_eq!(
        progress[0],
        "run: stopped: Codex failed T-1 (see report) after 1 call(s)"
    );
}

const LOCAL_GOAL: &str = r#"{"goal":"g","done_when":["d"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":5,"stop":"checkpoint",
  "routing":{"research":"local"},
  "tasks":[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":2}]}"#;

/// Drive the real Ollama adapter over a scripted process, the path a
/// child's stderr and a model's reply actually take.
fn run_local(runner: FakeCommand) -> (u8, String, String) {
    let agent = OllamaAgent::with_runner("test-model", Duration::from_secs(5), runner);
    let mut stdin = Cursor::new(Vec::new());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = {
        let mut streams = Streams {
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut stderr,
        };
        cli::run(&args(false, true), LOCAL_GOAL, agent, &mut streams).expect("run")
    };
    (
        code,
        String::from_utf8(stdout).expect("utf8"),
        String::from_utf8(stderr).expect("utf8"),
    )
}

#[test]
fn a_childs_stderr_excerpt_never_reaches_progress() {
    let (code, stdout, stderr) = run_local(FakeCommand::new(Ok(exit(1, "", SENTINEL))));

    assert_eq!(code, EXIT_FAILED);
    let json = Value::parse(&stdout).expect("one JSON document");
    let error = json
        .get("ended")
        .and_then(|e| e.get("error"))
        .and_then(|k| k.as_str())
        .expect("error");
    assert!(
        error.contains(SENTINEL),
        "the report keeps the excerpt: {error}"
    );
    assert!(!stderr.contains(SENTINEL), "{stderr}");
    assert_eq!(
        progress_lines(&stderr),
        vec!["run: stopped: Local failed T-1 (see report) after 1 call(s)"]
    );
}

#[test]
fn a_malformed_reply_never_reaches_progress() {
    // The parser echoes the offending kind in its message; that message
    // belongs to the report, not to stderr progress.
    let reply = format!(r#"{{"entries":[{{"kind":"{SENTINEL}","body":"x","refs":[]}}]}}"#);
    let (code, stdout, stderr) = run_local(FakeCommand::ok(&reply));

    assert_eq!(code, EXIT_FAILED);
    let json = Value::parse(&stdout).expect("one JSON document");
    let error = json
        .get("ended")
        .and_then(|e| e.get("error"))
        .and_then(|k| k.as_str())
        .expect("error");
    assert!(error.contains(SENTINEL), "{error}");
    assert!(!stderr.contains(SENTINEL), "{stderr}");
    assert_eq!(progress_lines(&stderr).len(), 1);
}
