//! The entry point over captured streams: stdout must stay a single JSON
//! object even when the run asks and takes an answer.

use std::io::Cursor;
use std::path::PathBuf;

use orch_core::board::{Confidence, EntryKind};
use orch_dispatch::fake::{FakeAgent, Reply};
use rusty_json::Value;
use rusty_orch::args::Args;
use rusty_orch::cli::{self, Streams, EXIT_BLOCKED, EXIT_FINISHED};

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
