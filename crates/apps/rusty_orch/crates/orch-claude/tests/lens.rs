//! The lens `Model` port over a scripted runner.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{error_envelope, exit, Scripted};
use orch_claude::{ClaudeAgent, SCRUBBED_ENV, TOOLS};
use orch_dispatch::ClassifiedError;
use orch_lens::{Allowed, Model, Question, Usage};

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

/// A plain envelope: `result` is the reply text, with a usage block.
fn reply(result: &str) -> String {
    let result = result.replace('"', "\\\"");
    format!(
        r#"{{"type":"result","subtype":"success","is_error":false,"result":"{result}","usage":{{"input_tokens":10,"cache_creation_input_tokens":20,"cache_read_input_tokens":30,"output_tokens":5}}}}"#
    )
}

fn agent(fake: Scripted) -> ClaudeAgent<Scripted> {
    ClaudeAgent::with_runner("/repo", fake).timeout(Duration::from_secs(30))
}

fn arg_after<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    let i = argv.iter().position(|a| a == flag)?;
    argv.get(i + 1).map(String::as_str)
}

#[test]
fn a_question_runs_without_tools_and_returns_posts_and_usage() {
    let mut a = agent(Scripted::ok(&reply(r#"{"posts":[]}"#)));

    let answer = a.ask(&question()).expect("answers");

    assert!(answer.posts.is_empty());
    assert_eq!(
        answer.usage,
        Usage {
            input_tokens: 60,
            output_tokens: 5,
            cached_input_tokens: 30,
        }
    );
    let calls = a.runner().inner.calls.lock().expect("lock");
    assert_eq!(calls.len(), 2, "probe, then run");
    let (argv, stdin, timeout) = &calls[1];
    assert_eq!(arg_after(argv, "--tools"), Some(""));
    assert_eq!(arg_after(argv, "--system-prompt"), Some("intent and rules"));
    assert!(!argv.iter().any(|a| a == "--json-schema"));
    assert_eq!(stdin, b"claim C-1");
    assert_eq!(*timeout, Duration::from_secs(30));
    let scrubbed = a.runner().inner.scrubbed.lock().expect("lock");
    assert_eq!(scrubbed[1], SCRUBBED_ENV);
    let cwds = a.runner().inner.cwds.lock().expect("lock");
    assert_eq!(cwds[1], Some(PathBuf::from("/repo")));
}

#[test]
fn a_read_only_lens_gets_the_read_only_tools() {
    let a = agent(Scripted::ok("")).lens_read_only();
    assert_eq!(arg_after(&a.lens_argv("p"), "--tools"), Some(TOOLS));
}

#[test]
fn a_non_conforming_reply_is_permanent() {
    for text in ["not json", r#"{"posts":[],"extra":1}"#] {
        let mut a = agent(Scripted::ok(&reply(text)));
        let err = a.ask(&question()).expect_err("refused");
        assert!(
            matches!(err, ClassifiedError::Permanent(_)),
            "{text}: {err:?}"
        );
    }
}

#[test]
fn a_missing_login_is_unavailable_and_nothing_runs() {
    let mut a = agent(Scripted::auth(Ok(exit(1, "", "not logged in"))));
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Unavailable(_)), "{err:?}");
    assert_eq!(a.runner().inner.calls.lock().expect("lock").len(), 1);
}

#[test]
fn a_reported_failure_is_classified() {
    let mut a = agent(Scripted::ok(&error_envelope("error", "429 rate limit")));
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Transient(_)), "{err:?}");
}
