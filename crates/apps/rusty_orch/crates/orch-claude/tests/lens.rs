//! The lens `Model` port over a scripted runner.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{error_envelope, exit, Scripted};
use orch_claude::{ClaudeAgent, SCRUBBED_ENV, TOOLS};
use orch_cli::Exit;
use orch_dispatch::ClassifiedError;
use orch_lens::{Allowed, Model, Question, Usage};

const USAGE: &str = r#"{"input_tokens":10,"cache_creation_input_tokens":20,"cache_read_input_tokens":30,"output_tokens":5}"#;

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

/// A plain envelope: `result` is the reply text, `usage` verbatim
/// (empty for none).
fn reply_with(result: &str, usage: &str) -> String {
    let result = result.replace('"', "\\\"");
    let usage = if usage.is_empty() {
        String::new()
    } else {
        format!(r#","usage":{usage}"#)
    };
    format!(
        r#"{{"type":"result","subtype":"success","is_error":false,"result":"{result}"{usage}}}"#
    )
}

fn reply(result: &str) -> String {
    reply_with(result, USAGE)
}

/// The account the operator's no-overage assertion names.
const ACCOUNT: &str = "nano@example.com";

fn status_as(method: &str, email: &str) -> Result<Exit, orch_cli::ExecError> {
    Ok(exit(
        0,
        &format!(r#"{{"loggedIn":true,"authMethod":"{method}","email":"{email}"}}"#),
        "",
    ))
}

fn status(method: &str) -> Result<Exit, orch_cli::ExecError> {
    status_as(method, ACCOUNT)
}

/// Logged in with a subscription; the run exits 0 with `stdout`.
fn subscribed(stdout: &str) -> Scripted {
    Scripted::both(status("claude.ai"), Ok(exit(0, stdout, "")))
}

fn agent(fake: Scripted) -> ClaudeAgent<Scripted> {
    ClaudeAgent::with_runner("/repo", fake)
        .timeout(Duration::from_secs(30))
        .lens_no_overage(ACCOUNT)
}

fn arg_after<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    let i = argv.iter().position(|a| a == flag)?;
    argv.get(i + 1).map(String::as_str)
}

fn permanent(mut a: ClaudeAgent<Scripted>) -> String {
    match a.ask(&question()) {
        Err(ClassifiedError::Permanent(m)) => m.0,
        other => panic!("expected Permanent, got {other:?}"),
    }
}

#[test]
fn a_question_runs_without_tools_and_returns_posts_and_usage() {
    let mut a = agent(subscribed(&reply(r#"{"posts":[]}"#)));

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
    assert_eq!(scrubbed[0], SCRUBBED_ENV);
    assert_eq!(scrubbed[1], SCRUBBED_ENV);
    let cwds = a.runner().inner.cwds.lock().expect("lock");
    assert_eq!(cwds[1], Some(PathBuf::from("/repo")));
}

#[test]
fn without_the_no_overage_assertion_nothing_runs() {
    let mut a = ClaudeAgent::with_runner("/repo", subscribed(&reply(r#"{"posts":[]}"#)));
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Unavailable(_)), "{err:?}");
    assert!(a.runner().inner.calls.lock().expect("lock").is_empty());
}

#[test]
fn another_or_an_unreported_account_is_unavailable_and_nothing_runs() {
    let no_email = Ok(exit(0, r#"{"loggedIn":true,"authMethod":"claude.ai"}"#, ""));
    for probe in [status_as("claude.ai", "someone@example.com"), no_email] {
        let mut a = agent(Scripted::both(
            probe.clone(),
            Ok(exit(0, &reply(r#"{"posts":[]}"#), "")),
        ));
        let err = a.ask(&question()).expect_err("refused");
        let ClassifiedError::Unavailable(m) = &err else {
            panic!("{probe:?}: {err:?}");
        };
        assert!(
            !m.0.contains("someone@"),
            "the reported account leaked: {m:?}"
        );
        assert_eq!(a.runner().inner.calls.lock().expect("lock").len(), 1);
    }
}

#[test]
fn the_account_matches_without_regard_to_case() {
    let mut a = agent(Scripted::both(
        status_as("claude.ai", "Nano@Example.com"),
        Ok(exit(0, &reply(r#"{"posts":[]}"#), "")),
    ));
    a.ask(&question()).expect("answers");
}

#[test]
fn a_headless_subscription_token_is_accepted() {
    let mut a = agent(Scripted::both(
        status("oauth_token"),
        Ok(exit(0, &reply(r#"{"posts":[]}"#), "")),
    ));
    a.ask(&question()).expect("answers");
}

#[test]
fn a_non_subscription_login_is_unavailable_and_nothing_runs() {
    let unreported = Ok(exit(0, r#"{"loggedIn":true}"#, ""));
    let not_json = Ok(exit(0, "Logged in", ""));
    for probe in [
        status("api_key"),
        status("api_key_helper"),
        status("third_party"),
        status("none"),
        status("something_new"),
        unreported,
        not_json,
    ] {
        let mut a = agent(Scripted::both(
            probe.clone(),
            Ok(exit(0, &reply(r#"{"posts":[]}"#), "")),
        ));
        let err = a.ask(&question()).expect_err("refused");
        assert!(
            matches!(err, ClassifiedError::Unavailable(_)),
            "{probe:?}: {err:?}"
        );
        assert_eq!(a.runner().inner.calls.lock().expect("lock").len(), 1);
    }
}

#[test]
fn a_read_only_lens_gets_the_read_only_tools() {
    let a = agent(Scripted::ok("")).lens_read_only();
    assert_eq!(arg_after(&a.lens_argv("p"), "--tools"), Some(TOOLS));
}

#[test]
fn a_non_conforming_reply_is_permanent() {
    for text in ["not json", r#"{"posts":[],"extra":1}"#] {
        permanent(agent(subscribed(&reply(text))));
    }
}

#[test]
fn unmeasured_usage_is_permanent() {
    for usage in [
        "",
        r#"{"output_tokens":5}"#,
        r#"{"input_tokens":10}"#,
        r#"{"input_tokens":"10","output_tokens":5}"#,
        r#"{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":-1}"#,
    ] {
        let m = permanent(agent(subscribed(&reply_with(r#"{"posts":[]}"#, usage))));
        assert!(m.contains("usage"), "{usage}: {m}");
    }
}

#[test]
fn a_reply_that_is_not_utf8_is_permanent() {
    let mut stdout = reply(r#"{"posts":[]}"#).into_bytes();
    let at = stdout
        .iter()
        .position(|b| *b == b'p')
        .expect("a byte to break");
    stdout[at] = 0xff;
    let run = Exit {
        status: 0,
        stdout,
        stderr: Vec::new(),
    };
    let m = permanent(agent(Scripted::both(status("claude.ai"), Ok(run))));
    assert!(m.contains("UTF-8"), "{m}");
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
    let mut a = agent(subscribed(&error_envelope("error", "429 rate limit")));
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Transient(_)), "{err:?}");
}
