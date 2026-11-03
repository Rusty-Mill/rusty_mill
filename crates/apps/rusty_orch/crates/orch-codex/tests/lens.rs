//! The lens `Model` port over a scripted runner.

mod common;

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use common::{exit, FakeCommand, ReplyFile};
use orch_cli::{CommandRunner, ExecError, Exit};
use orch_codex::{CodexAgent, LOGIN_ARGV, SCRUBBED_ENV};
use orch_dispatch::ClassifiedError;
use orch_lens::{Allowed, Model, Question, Usage};

const EVENTS: &str = "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":60,\"output_tokens\":7}}\n";

/// Answers the login probe with `probe` and every run through `run`.
struct Probed {
    probe: Exit,
    probes: Mutex<Vec<Vec<String>>>,
    run: ReplyFile,
}

impl CommandRunner for Probed {
    fn run_in(
        &self,
        cwd: Option<&Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError> {
        if argv == LOGIN_ARGV {
            assert_eq!(remove_env, SCRUBBED_ENV);
            self.probes.lock().expect("lock").push(argv.to_vec());
            return Ok(self.probe.clone());
        }
        self.run.run_in(cwd, argv, stdin, timeout, remove_env)
    }
}

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

const CHATGPT: &str = "Logged in using ChatGPT\n";

fn runner(probe: Exit, run: ReplyFile) -> Probed {
    Probed {
        probe,
        probes: Mutex::new(Vec::new()),
        run,
    }
}

fn replying(reply: &str, events: &str) -> Probed {
    runner(
        exit(0, "", CHATGPT),
        ReplyFile {
            inner: FakeCommand::ok(events),
            reply: Some(reply.to_owned()),
        },
    )
}

fn agent(fake: Probed) -> CodexAgent<Probed> {
    CodexAgent::with_runner("/repo", fake).timeout(Duration::from_secs(30))
}

fn arg_after<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    let i = argv.iter().position(|a| a == flag)?;
    argv.get(i + 1).map(String::as_str)
}

fn runs(a: &CodexAgent<Probed>) -> usize {
    a.runner().run.inner.calls.lock().expect("lock").len()
}

#[test]
fn a_question_runs_in_a_scratch_dir_and_returns_posts_and_usage() {
    let mut a = agent(replying(r#"{"posts":[]}"#, EVENTS));

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
    assert_eq!(a.runner().probes.lock().expect("lock").len(), 1);
    let calls = a.runner().run.inner.calls.lock().expect("lock");
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
    let scrubbed = a.runner().run.inner.scrubbed.lock().expect("lock");
    assert_eq!(scrubbed[0], SCRUBBED_ENV);
}

#[test]
fn a_non_subscription_login_is_unavailable_and_nothing_runs() {
    for probe in [
        exit(0, "", "Logged in using an API key - sk-proj-***ABCDE\n"),
        exit(1, "", "Not logged in\n"),
        exit(0, "", "something new\n"),
        exit(1, "", CHATGPT),
    ] {
        let mut a = agent(runner(probe.clone(), ReplyFile::ok(r#"{"posts":[]}"#)));
        let err = a.ask(&question()).expect_err("refused");
        let ClassifiedError::Unavailable(m) = &err else {
            panic!("{probe:?}: {err:?}");
        };
        assert!(!m.0.contains("sk-proj"), "{}", m.0);
        assert_eq!(runs(&a), 0, "{probe:?}");
    }
}

#[test]
fn a_read_only_lens_runs_in_the_repo() {
    let mut a = agent(replying(r#"{"posts":[]}"#, EVENTS)).lens_read_only();
    a.ask(&question()).expect("answers");
    let calls = a.runner().run.inner.calls.lock().expect("lock");
    assert_eq!(arg_after(&calls[0].0, "-C"), Some("/repo"));
}

#[test]
fn a_non_conforming_reply_is_permanent() {
    for text in ["not json", r#"{"posts":[],"extra":1}"#] {
        let mut a = agent(replying(text, EVENTS));
        let err = a.ask(&question()).expect_err("refused");
        assert!(
            matches!(err, ClassifiedError::Permanent(_)),
            "{text}: {err:?}"
        );
    }
}

#[test]
fn unmeasured_usage_is_permanent() {
    for events in [
        "",
        "{\"type\":\"turn.completed\",\"usage\":{\"output_tokens\":7}}\n",
    ] {
        let mut a = agent(replying(r#"{"posts":[]}"#, events));
        let err = a.ask(&question()).expect_err("refused");
        assert!(
            matches!(err, ClassifiedError::Permanent(_)),
            "{events}: {err:?}"
        );
    }
}

#[test]
fn a_rate_limit_is_transient() {
    let mut a = agent(runner(
        exit(0, "", CHATGPT),
        ReplyFile::failing(Ok(exit(1, "", "429 Too Many Requests"))),
    ));
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Transient(_)), "{err:?}");
}

#[test]
fn no_last_message_is_transient() {
    let mut a = agent(runner(exit(0, "", CHATGPT), ReplyFile::silent()));
    let err = a.ask(&question()).expect_err("refused");
    assert!(matches!(err, ClassifiedError::Transient(_)), "{err:?}");
}
