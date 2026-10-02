mod common;

use std::time::Duration;

use common::{exit, fixture, task, text, Call, FakeCommand, REPLY_ONE};
use orch_cli::ExecError;
use orch_core::board::{Author, Confidence, EntryKind, NewEntry};
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::GoalId;
use orch_dispatch::AgentRunner;
use orch_ollama::OllamaAgent;

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
    // Ollama has no API-key path: the child's environment is untouched.
    assert_eq!(
        a.runner().scrubbed.lock().expect("lock").as_slice(),
        &[Vec::<String>::new()]
    );
}

#[test]
fn wrong_agent_is_refused_without_running_anything() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::ok(REPLY_ONE));

    let err = a
        .run(Agent::Codex, task(&plan), &board)
        .expect_err("refused");

    assert!(err
        .message()
        .contains("cannot serve Research through Codex"));
    assert!(err.is_permanent());
    assert!(a_calls(&a).is_empty());
}

#[test]
fn implement_is_refused_without_running_anything() {
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
    let mut a = agent(FakeCommand::ok(REPLY_ONE));

    let err = a
        .run(Agent::Local, plan.get(id).expect("task"), &board)
        .expect_err("unsupported");

    assert!(err
        .message()
        .contains("cannot serve Implement through Local"));
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

    assert!(err.message().starts_with("ollama exited with status 1"));
    assert!(err.message().contains("model not found"));
    assert!(
        err.message().chars().count() < 300,
        "stderr must be truncated"
    );
    assert!(!err.message().contains('\n'));
}

#[test]
fn timeout_maps_to_agent_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::new(Err(ExecError::Timeout(TIMEOUT))));

    let err = a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("timeout");

    assert!(err.message().contains("exceeded 30s"));
}

#[test]
fn overflow_and_spawn_failure_map_to_agent_errors() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::new(Err(ExecError::StdoutOverflow)));
    assert!(a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("overflow")
        .message()
        .contains("more than"));

    let mut a = agent(FakeCommand::new(Err(ExecError::Spawn(
        "No such file".into(),
    ))));
    assert!(a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("spawn")
        .message()
        .contains("could not start"));
}

#[test]
fn empty_stdout_is_an_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::ok("  \n"));

    let err = a.run(Agent::Local, task(&plan), &board).expect_err("empty");

    assert!(err.message().contains("wrote nothing"));
}

#[test]
fn garbage_stdout_is_a_parse_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(FakeCommand::ok("I cannot help with that."));

    let err = a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("garbage");

    assert!(err.message().contains("not JSON"));
    assert!(!err.is_permanent(), "the next sample may well parse");
}

fn a_calls(a: &OllamaAgent<FakeCommand>) -> Vec<Call> {
    a.runner().calls.lock().expect("lock").clone()
}

/// Issue #448, through the adapter: the prompt a resumed card sends carries
/// the answer to its earlier question.
#[test]
fn resumed_card_prompt_carries_the_answer() {
    let (plan, mut board, referenced, _) = fixture();
    let card = task(&plan);
    let successor = board
        .append(NewEntry {
            task: None,
            author: Author::Human,
            kind: EntryKind::Finding {
                confidence: Confidence::High,
            },
            body: text("The live referenced finding."),
            refs: vec![],
            supersedes: Some(referenced),
        })
        .expect("successor");
    let question = board
        .append(NewEntry {
            task: Some(card.id()),
            author: Author::Agent(Agent::Local),
            kind: EntryKind::Question,
            body: text("Which branch is the baseline?"),
            refs: vec![],
            supersedes: None,
        })
        .expect("question");
    board
        .append(NewEntry {
            task: None,
            author: Author::Human,
            kind: EntryKind::Answer { to: question },
            body: text("Use main as of this morning."),
            refs: vec![],
            supersedes: None,
        })
        .expect("answer");
    let mut a = agent(FakeCommand::ok(REPLY_ONE));

    a.run(Agent::Local, card, &board).expect("ok");

    let calls = a_calls(&a);
    let prompt = String::from_utf8(calls[0].1.clone()).expect("utf8");
    assert!(prompt.contains("Use main as of this morning."));
    assert!(prompt.contains(&format!("{successor} [Finding")));
    assert!(prompt.contains("The live referenced finding."));
    assert!(!prompt.contains("Plan::start rejects the author."));
}
