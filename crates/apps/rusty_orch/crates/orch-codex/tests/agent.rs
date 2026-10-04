//! Layer 2: the adapter over a fake runner.

mod common;

use std::time::Duration;

use common::{exit, fixture, task, text, ReplyFile, REPLY_ONE};
use orch_cli::ExecError;
use orch_codex::CodexAgent;
use orch_core::board::{Author, Confidence, EntryKind, NewEntry};
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::GoalId;
use orch_dispatch::{AgentRunner, ClassifiedError};

fn agent(fake: ReplyFile) -> CodexAgent<ReplyFile> {
    CodexAgent::with_runner("/repo", fake).timeout(Duration::from_secs(30))
}

#[test]
fn success_reads_the_last_message_file_and_scrubs_the_api_key() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(ReplyFile::ok(REPLY_ONE));

    let out = a.run(Agent::Codex, task(&plan), &board).expect("ok");

    assert_eq!(out.len(), 1);
    let calls = a.runner().inner.calls.lock().expect("lock");
    let (argv, stdin, timeout) = &calls[0];
    assert_eq!(argv[0..4], ["codex", "exec", "--sandbox", "read-only"]);
    assert_eq!(*timeout, Duration::from_secs(30));
    let prompt = String::from_utf8(stdin.clone()).expect("utf8");
    assert!(prompt.contains("OUTPUT FORMAT"));
    assert!(prompt.contains("Plan::start rejects the author."));
    let scrubbed = a.runner().inner.scrubbed.lock().expect("lock");
    assert_eq!(scrubbed[0], ["OPENAI_API_KEY"]);
    // Scratch files are gone after the run.
    let reply_path = &argv[argv.len() - 2];
    assert!(!std::path::Path::new(reply_path).exists());
}

#[test]
fn wrong_agent_is_refused_without_running() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(ReplyFile::ok(REPLY_ONE));
    let err = a
        .run(Agent::Local, task(&plan), &board)
        .expect_err("refused");
    assert!(err
        .0
        .as_str()
        .contains("cannot serve Research through Local"));
    assert!(a.runner().inner.calls.lock().expect("lock").is_empty());
}

#[test]
fn implement_is_refused_before_scratch_or_process_work() {
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
    let mut a = agent(ReplyFile::ok(REPLY_ONE));

    let err = a
        .run(Agent::Codex, plan.get(id).expect("task"), &board)
        .expect_err("unsupported");

    assert!(err
        .0
        .as_str()
        .contains("cannot serve Implement through Codex"));
    assert!(a.runner().inner.calls.lock().expect("lock").is_empty());
}

#[test]
fn not_logged_in_is_named() {
    let (plan, board, _, _) = fixture();
    let stderr = "ERROR: Reconnecting... 5/5\nERROR: unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses\n";
    let mut a = agent(ReplyFile::failing(Ok(exit(1, "", stderr))));
    let err = a.run(Agent::Codex, task(&plan), &board).expect_err("401");
    assert!(
        err.0.as_str().starts_with("codex: not logged in"),
        "{}",
        err.0.as_str()
    );
    assert!(err.0.as_str().contains("codex login"));
    assert!(!err.0.as_str().contains('\n'));
}

#[test]
fn not_logged_in_is_an_unavailable_prerequisite() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(ReplyFile::failing(Ok(exit(
        1,
        "",
        "ERROR: unexpected status 401 Unauthorized",
    ))));

    let failure = a
        .run_classified(Agent::Codex, task(&plan), &board)
        .expect_err("401");

    assert!(matches!(failure, ClassifiedError::Unavailable(_)));
}

#[test]
fn rate_limit_is_named_and_distinct_from_login() {
    let (plan, board, _, _) = fixture();
    for stderr in [
        "ERROR: unexpected status 429 Too Many Requests",
        "ERROR: rate_limit_reached: weekly limit hit",
        "ERROR: usage_limit_reached",
    ] {
        let mut a = agent(ReplyFile::failing(Ok(exit(1, "", stderr))));
        let err = a.run(Agent::Codex, task(&plan), &board).expect_err("429");
        assert!(
            err.0.as_str().starts_with("codex: rate limited"),
            "{}",
            err.0.as_str()
        );
        assert!(!err.0.as_str().contains("not logged in"));
    }
}

#[test]
fn sandbox_refusal_is_a_normal_reply_not_an_error() {
    // Codex returns a denied command to the model; the model reports it.
    let (plan, board, _, _) = fixture();
    let reply = r#"{"entries":[{"kind":"assumption","body":"Could not run `git log`: the sandbox denied it, so history is unverified.","refs":[]}]}"#;
    let mut a = agent(ReplyFile::ok(reply));
    let out = a.run(Agent::Codex, task(&plan), &board).expect("parses");
    assert_eq!(out.len(), 1);
}

#[test]
fn other_non_zero_exit_keeps_the_status() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(ReplyFile::failing(Ok(exit(
        2,
        "",
        "error: unexpected argument",
    ))));
    let err = a
        .run(Agent::Codex, task(&plan), &board)
        .expect_err("exit 2");
    assert!(
        err.0.as_str().starts_with("codex exited with status 2"),
        "{}",
        err.0.as_str()
    );
}

#[test]
fn timeout_maps_to_agent_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(ReplyFile::failing(Err(ExecError::Timeout(
        Duration::from_secs(30),
    ))));
    let err = a
        .run(Agent::Codex, task(&plan), &board)
        .expect_err("timeout");
    assert!(err.0.as_str().contains("exceeded 30s"));
}

#[test]
fn malformed_reply_is_a_parse_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(ReplyFile::ok("Sure! Here is what I found."));
    let err = a.run(Agent::Codex, task(&plan), &board).expect_err("prose");
    assert!(err.0.as_str().contains("not JSON"));
}

#[test]
fn missing_last_message_file_is_an_error() {
    let (plan, board, _, _) = fixture();
    let mut a = agent(ReplyFile::silent());
    let err = a
        .run(Agent::Codex, task(&plan), &board)
        .expect_err("no file");
    assert!(err.0.as_str().contains("wrote no last message"));
}

#[test]
fn resumed_card_prompt_carries_answers_and_live_successors() {
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
            author: Author::Agent(Agent::Codex),
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
    let mut a = agent(ReplyFile::ok(REPLY_ONE));

    a.run(Agent::Codex, card, &board).expect("ok");

    let calls = a.runner().inner.calls.lock().expect("lock");
    let prompt = String::from_utf8(calls[0].1.clone()).expect("utf8");
    assert!(prompt.contains("Use main as of this morning."));
    assert!(prompt.contains(&format!("{successor} [Finding")));
    assert!(prompt.contains("The live referenced finding."));
    assert!(!prompt.contains("Plan::start rejects the author."));
}

#[test]
fn best_effort_withdraws_questions_from_prompt_and_parser() {
    use orch_core::goal::StopRule;
    let (plan, board, _, _) = fixture();
    let question = r#"{"entries":[{"kind":"question","body":"which branch?","refs":[]}]}"#;
    let mut a = agent(ReplyFile::ok(question)).with_stop_rule(StopRule::BestEffort);

    let err = a
        .run_classified(Agent::Codex, task(&plan), &board)
        .expect_err("questions are withdrawn under best effort");
    assert!(
        matches!(&err, ClassifiedError::Transient(e) if e.0.contains("question")),
        "{err:?}"
    );
    let calls = a.runner().inner.calls.lock().expect("lock");
    let prompt = String::from_utf8(calls[0].1.clone()).expect("utf8");
    assert!(prompt.contains("This run is best-effort"));
    assert!(prompt.contains("kind is one of: finding, assumption."));
    drop(calls);

    // Under checkpoint the same reply is a question the dispatcher can block on.
    let mut a = agent(ReplyFile::ok(question));
    let out = a.run(Agent::Codex, task(&plan), &board).expect("ok");
    assert_eq!(out[0].kind, EntryKind::Question);
}
