//! Layer 2: the adapter over a scripted runner.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{envelope, error_envelope, exit, fixture, task, text, Scripted, REPLY_ONE};
use orch_claude::{ClaudeAgent, SCRUBBED_ENV};
use orch_cli::ExecError;
use orch_core::board::EntryKind;
use orch_core::goal::StopRule;
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::GoalId;
use orch_dispatch::{AgentRunner, ClassifiedError};

fn agent(fake: Scripted) -> ClaudeAgent<Scripted> {
    ClaudeAgent::with_runner("/repo", fake).timeout(Duration::from_secs(30))
}

#[test]
fn success_probes_the_login_then_runs_in_the_repo_with_the_key_scrubbed() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::ok(&envelope(REPLY_ONE)));

    let out = a.run(Agent::Claude, task(&plan), &board).expect("ok");

    assert_eq!(out.len(), 1);
    let calls = a.runner().inner.calls.lock().expect("lock");
    assert_eq!(calls.len(), 2, "probe, then run");
    let (probe, probe_stdin, probe_timeout) = &calls[0];
    assert_eq!(probe, &["claude", "auth", "status"]);
    assert!(probe_stdin.is_empty());
    assert_eq!(*probe_timeout, orch_claude::AUTH_TIMEOUT);
    let (argv, stdin, timeout) = &calls[1];
    assert_eq!(argv[0..4], ["claude", "-p", "--output-format", "json"]);
    assert_eq!(*timeout, Duration::from_secs(30));
    let prompt = String::from_utf8(stdin.clone()).expect("utf8");
    assert!(prompt.contains("OUTPUT FORMAT"));
    assert!(prompt.contains("Plan::start rejects the author."));
    let scrubbed = a.runner().inner.scrubbed.lock().expect("lock");
    assert_eq!(scrubbed[0], SCRUBBED_ENV);
    assert_eq!(scrubbed[1], SCRUBBED_ENV);
    let cwds = a.runner().inner.cwds.lock().expect("lock");
    assert_eq!(cwds[0], Some(PathBuf::from("/repo")));
    assert_eq!(cwds[1], Some(PathBuf::from("/repo")));
}

#[test]
fn wrong_agent_is_refused_without_running() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::ok(&envelope(REPLY_ONE)));
    let err = a
        .run(Agent::Codex, task(&plan), &board)
        .expect_err("refused");
    assert!(err
        .0
        .as_str()
        .contains("cannot serve Research through Codex"));
    assert!(a.runner().inner.calls.lock().expect("lock").is_empty());
}

#[test]
fn implement_is_refused_before_any_process() {
    let (_, board, _, _) = fixture();
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan
        .add(TaskSpec {
            role: Role::Implement,
            instruction: text("implement it"),
            acceptance: vec![text("done")],
            refs: vec![],
            depends_on: vec![],
            max_calls: std::num::NonZeroU32::new(3).expect("non-zero"),
        })
        .expect("add");
    let mut a = agent(Scripted::ok(&envelope(REPLY_ONE)));

    let err = a
        .run_classified(Agent::Claude, plan.get(id).expect("task"), &board)
        .expect_err("unsupported");

    assert!(
        matches!(&err, ClassifiedError::Permanent(e) if e.0.contains("cannot serve Implement through Claude"))
    );
    assert!(a.runner().inner.calls.lock().expect("lock").is_empty());
}

#[test]
fn logged_out_probe_is_unavailable_and_skips_the_run() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::auth(Ok(exit(
        1,
        r#"{"loggedIn":false,"authMethod":"none"}"#,
        "",
    ))));

    let failure = a
        .run_classified(Agent::Claude, task(&plan), &board)
        .expect_err("logged out");

    assert!(matches!(&failure, ClassifiedError::Unavailable(e)
        if e.0.starts_with("claude: not logged in") && e.0.contains("claude auth login")));
    assert_eq!(a.runner().inner.calls.lock().expect("lock").len(), 1);
}

#[test]
fn probe_that_cannot_run_is_transient() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::auth(Err(ExecError::Spawn(
        "No such file or directory".to_owned(),
    ))));
    let failure = a
        .run_classified(Agent::Claude, task(&plan), &board)
        .expect_err("no binary");
    assert!(
        matches!(&failure, ClassifiedError::Transient(e) if e.0.contains("could not start process"))
    );
}

#[test]
fn login_failure_inside_the_envelope_is_unavailable() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::ok(&error_envelope(
        "error_during_execution",
        "Not logged in · Please run /login",
    )));

    let failure = a
        .run_classified(Agent::Claude, task(&plan), &board)
        .expect_err("login");

    assert!(matches!(&failure, ClassifiedError::Unavailable(e)
        if e.0.starts_with("claude: not logged in") && e.0.contains("/login")));
}

#[test]
fn rate_limit_is_named_and_distinct_from_login() {
    let (plan, board, _, _) = fixture();
    for result in [
        "429 Too Many Requests",
        "You have hit your usage limit",
        "API Error: rate_limit_error",
        "Overloaded",
    ] {
        let mut a = agent(Scripted::ok(&error_envelope(
            "error_during_execution",
            result,
        )));
        let err = a
            .run(Agent::Claude, task(&plan), &board)
            .expect_err("limited");
        assert!(
            err.0.as_str().starts_with("claude: rate limited"),
            "{}",
            err.0.as_str()
        );
        assert!(!err.0.as_str().contains("not logged in"));
    }
}

#[test]
fn other_envelope_error_keeps_status_and_subtype() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::main(Ok(exit(
        1,
        &error_envelope("error_max_turns", "Reached max turns (20)"),
        "",
    ))));
    let err = a
        .run(Agent::Claude, task(&plan), &board)
        .expect_err("max turns");
    assert!(
        err.0
            .as_str()
            .starts_with("claude exited with status 1 (error_max_turns)"),
        "{}",
        err.0.as_str()
    );
    assert!(err.0.as_str().contains("Reached max turns"));
    assert!(!err.0.as_str().contains('\n'));
}

#[test]
fn non_zero_exit_without_an_envelope_reports_stderr() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::main(Ok(exit(
        2,
        "",
        "error: unknown option '--json-schema'",
    ))));
    let err = a
        .run(Agent::Claude, task(&plan), &board)
        .expect_err("exit 2");
    assert!(
        err.0.as_str().starts_with("claude exited with status 2"),
        "{}",
        err.0.as_str()
    );
    assert!(err.0.as_str().contains("unknown option"));
}

#[test]
fn prose_instead_of_an_envelope_is_transient() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::ok("Sure! Here is what I found."));
    let failure = a
        .run_classified(Agent::Claude, task(&plan), &board)
        .expect_err("prose");
    assert!(
        matches!(&failure, ClassifiedError::Transient(e) if e.0.contains("no result envelope"))
    );
}

#[test]
fn envelope_without_structured_output_is_transient() {
    let (plan, board, _, _) = fixture();
    let bare = r#"{"type":"result","subtype":"success","is_error":false,"result":"I could not form the object."}"#;
    let mut a = agent(Scripted::ok(bare));
    let failure = a
        .run_classified(Agent::Claude, task(&plan), &board)
        .expect_err("no object");
    assert!(matches!(&failure, ClassifiedError::Transient(e)
        if e.0.contains("no structured output") && e.0.contains("could not form")));
}

#[test]
fn structured_output_still_goes_through_the_parser() {
    let (plan, board, _, _) = fixture();
    let off_protocol = r#"{"entries":[{"kind":"poem","body":"roses","refs":[]}]}"#;
    let mut a = agent(Scripted::ok(&envelope(off_protocol)));
    let err = a
        .run(Agent::Claude, task(&plan), &board)
        .expect_err("bad kind");
    assert!(err.0.as_str().contains("kind"), "{}", err.0.as_str());
}

#[test]
fn timeout_maps_to_agent_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(Scripted::main(Err(ExecError::Timeout(
        Duration::from_secs(30),
    ))));
    let err = a
        .run(Agent::Claude, task(&plan), &board)
        .expect_err("timeout");
    assert!(err.0.as_str().contains("exceeded 30s"));
}

#[test]
fn best_effort_withdraws_questions_from_prompt_schema_and_parser() {
    let (plan, board, _, _) = fixture();
    let question = r#"{"entries":[{"kind":"question","body":"which branch?","refs":[]}]}"#;
    let mut a = agent(Scripted::ok(&envelope(question))).with_stop_rule(StopRule::BestEffort);

    let err = a
        .run_classified(Agent::Claude, task(&plan), &board)
        .expect_err("questions are withdrawn under best effort");
    assert!(
        matches!(&err, ClassifiedError::Transient(e) if e.0.contains("question")),
        "{err:?}"
    );
    let calls = a.runner().inner.calls.lock().expect("lock");
    let (argv, stdin, _) = &calls[1];
    let prompt = String::from_utf8(stdin.clone()).expect("utf8");
    assert!(prompt.contains("This run is best-effort"));
    assert!(prompt.contains("kind is one of: finding, assumption."));
    assert!(!argv[argv.len() - 1].contains("\"question\""));
    drop(calls);

    // Under checkpoint the same reply is a question the dispatcher can block on.
    let mut a = agent(Scripted::ok(&envelope(question)));
    let out = a.run(Agent::Claude, task(&plan), &board).expect("ok");
    assert_eq!(out[0].kind, EntryKind::Question);
}
