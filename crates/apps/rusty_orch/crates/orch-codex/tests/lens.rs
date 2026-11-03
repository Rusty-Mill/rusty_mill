//! The lens `Model` port over a scripted runner.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{exit, FakeCommand, ReplyFile};
use orch_codex::{CodexAgent, SCRUBBED_ENV};
use orch_dispatch::ClassifiedError;
use orch_lens::{Allowed, Model, Question, Usage};

const EVENTS: &str = "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":60,\"output_tokens\":7}}\n";

fn question() -> Question {
    Question {
        prefix: "intent and rules".to_owned(),
        body: "claim C-1".to_owned(),
        allowed: Allowed {
            evidence: None,
            claims: None,
        },
        max_output_tokens: 256,
    }
}

fn replying(reply: &str) -> ReplyFile {
    ReplyFile {
        inner: FakeCommand::ok(EVENTS),
        reply: Some(reply.to_owned()),
    }
}

fn agent(fake: ReplyFile) -> CodexAgent<ReplyFile> {
    CodexAgent::with_runner("/repo", fake).timeout(Duration::from_secs(30))
}

fn arg_after<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    let i = argv.iter().position(|a| a == flag)?;
    argv.get(i + 1).map(String::as_str)
}

#[test]
fn a_question_runs_in_a_scratch_dir_and_returns_posts_and_usage() {
    let mut a = agent(replying(r#"{"posts":[]}"#));

    let answer = a.ask(&question()).expect("answers");

    assert!(answer.posts.is_empty());
    assert_eq!(
        answer.usage,
        Usage {
            input_tokens: 100,
            output_tokens: 7,
            cached_input_tokens: 60,
        }
    );
    let calls = a.runner().inner.calls.lock().expect("lock");
    let (argv, stdin, timeout) = &calls[0];
    assert_eq!(argv[0..4], ["codex", "exec", "--sandbox", "read-only"]);
    let cwd = arg_after(argv, "-C").expect("-C");
    assert_ne!(cwd, "/repo");
    assert!(!Path::new(cwd).exists(), "scratch dir removed");
    assert!(argv.iter().any(|a| a == "--json"));
    assert!(argv.iter().any(|a| a == "--skip-git-repo-check"));
    assert!(!argv.iter().any(|a| a == "--output-schema"));
    assert_eq!(argv.last().map(String::as_str), Some("-"));
    assert_eq!(stdin, b"intent and rules\n\nclaim C-1");
    assert_eq!(*timeout, Duration::from_secs(30));
    let scrubbed = a.runner().inner.scrubbed.lock().expect("lock");
    assert_eq!(scrubbed[0], SCRUBBED_ENV);
}

#[test]
fn a_read_only_lens_runs_in_the_repo() {
    let mut a = agent(replying(r#"{"posts":[]}"#)).lens_read_only();
    a.ask(&question()).expect("answers");
    let calls = a.runner().inner.calls.lock().expect("lock");
    assert_eq!(arg_after(&calls[0].0, "-C"), Some("/repo"));
}

#[test]
fn a_non_conforming_reply_is_permanent() {
    for text in ["not json", r#"{"posts":[],"extra":1}"#] {
        let mut a = agent(replying(text));
        let err = a.ask(&question()).expect_err("refused");
        assert!(
            matches!(err, ClassifiedError::Permanent(_)),
            "{text}: {err:?}"
        );
    }
}

#[test]
fn a_missing_login_is_unavailable() {
    let mut a = agent(ReplyFile::failing(Ok(exit(1, "", "401 Unauthorized"))));
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Unavailable(_)), "{err:?}");
}

#[test]
fn no_last_message_is_transient() {
    let mut a = agent(ReplyFile::silent());
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Transient(_)), "{err:?}");
}
