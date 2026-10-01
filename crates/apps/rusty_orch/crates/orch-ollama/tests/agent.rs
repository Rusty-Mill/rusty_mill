mod common;

use std::time::Duration;

use common::{exit, fixture, task, Call, FakeCommand, REPLY_ONE};
use orch_core::task::Agent;
use orch_dispatch::AgentRunner;
use orch_ollama::{ExecError, OllamaAgent};

const TIMEOUT: Duration = Duration::from_secs(30);

fn agent(fake: FakeCommand) -> OllamaAgent<FakeCommand> {
    OllamaAgent::with_runner("test-model", TIMEOUT, fake)
}

#[test]
fn success_path_runs_fixed_argv_with_prompt_on_stdin() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::ok(REPLY_ONE));

    let out = a.run(Agent::Local, task(&plan), &board).expect("ok");

    assert_eq!(out.len(), 1);
    let calls = a_calls(&a);
    assert_eq!(calls.len(), 1);
    let (argv, stdin, timeout) = &calls[0];
    assert_eq!(argv, &["ollama", "run", "test-model", "--format", "json"]);
    assert_eq!(*timeout, TIMEOUT);
    let prompt = String::from_utf8(stdin.clone()).expect("utf8");
    assert!(prompt.contains("OUTPUT FORMAT"));
    assert!(prompt.contains("Plan::start rejects the author."));
}

#[test]
fn wrong_agent_is_refused_without_running_anything() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::ok(REPLY_ONE));

    let err = a
        .run(Agent::Codex, task(&plan), &board)
        .expect_err("refused");

    assert!(err.0.contains("serves Local, not Codex"));
    assert!(a_calls(&a).is_empty());
}

#[test]
fn non_zero_exit_maps_to_agent_error_with_truncated_stderr() {
    let (plan, board, _, _) = fixture();
    let stderr = "model not found\n".repeat(50);
    let mut a = agent(FakeCommand::new(Ok(exit(1, "", &stderr))));

    let err = a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("exit 1");

    assert!(err.0.starts_with("ollama exited with status 1"));
    assert!(err.0.contains("model not found"));
    assert!(err.0.chars().count() < 300, "stderr must be truncated");
    assert!(!err.0.contains('\n'));
}

#[test]
fn timeout_maps_to_agent_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::new(Err(ExecError::Timeout(TIMEOUT))));

    let err = a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("timeout");

    assert!(err.0.contains("exceeded 30s"));
}

#[test]
fn overflow_and_spawn_failure_map_to_agent_errors() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::new(Err(ExecError::StdoutOverflow)));
    assert!(a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("overflow")
        .0
        .contains("more than"));

    let mut a = agent(FakeCommand::new(Err(ExecError::Spawn(
        "No such file".into(),
    ))));
    assert!(a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("spawn")
        .0
        .contains("could not start"));
}

#[test]
fn empty_stdout_is_an_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::ok("  \n"));

    let err = a.run(Agent::Local, task(&plan), &board).expect_err("empty");

    assert!(err.0.contains("wrote nothing"));
}

#[test]
fn garbage_stdout_is_a_parse_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::ok("I cannot help with that."));

    let err = a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("garbage");

    assert!(err.0.contains("not JSON"));
}

fn a_calls(a: &OllamaAgent<FakeCommand>) -> Vec<Call> {
    a.runner().calls.lock().expect("lock").clone()
}
