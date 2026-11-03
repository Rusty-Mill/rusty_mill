//! The lens runner's [`Model`] port over `codex exec` (#594).
//!
//! `codex exec` has no before-call bound on its agent loop, so this adapter
//! is admitted only at zero price, and only where no call can add a charge.
//! Three separate gates, all fail closed: the operator's assertion that
//! the account cannot bill beyond its plan ([`CodexAgent::lens_no_overage`]),
//! the operator's assertion that everything the host lets Codex read is
//! cleared for it ([`CodexAgent::lens_host_cleared`]; Codex has no
//! tool-less mode and its read-only sandbox reads the whole filesystem),
//! and a ChatGPT login on every question. The login check establishes the
//! auth method, not billing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use orch_cli::{excerpt, CommandRunner};
use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{parse, Answer, Model, Question, Usage};
use rusty_json::Value;

use crate::agent::{CodexAgent, RUN_SEQ, SCRUBBED_ENV};

/// The login probe run before every lens question.
pub const LOGIN_ARGV: &[&str] = &["codex", "login", "status"];

/// `codex login status` reads local state only; a minute is generous.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(60);

/// What `codex login status` prints for a ChatGPT subscription login
/// (codex-cli 0.160.0). An API-key login, no login, or anything else is
/// refused.
const SUBSCRIPTION_LOGIN: &str = "Logged in using ChatGPT";

impl<C: CommandRunner> CodexAgent<C> {
    /// The argv for one lens question: the base argv in the repository
    /// root, JSONL events on stdout (for usage), the last message to
    /// `reply`, the prompt on stdin. No schema: the reply is checked by
    /// [`orch_lens::parse`]. Public so tests and docs can pin it.
    pub fn lens_argv(&self, reply: &Path) -> Vec<String> {
        let mut argv = self.base_argv(self.repo_root());
        argv.push("--json".to_owned());
        argv.push("--output-last-message".to_owned());
        argv.push(reply.display().to_string());
        argv.push("-".to_owned());
        argv
    }
}

impl<C: CommandRunner> Model for CodexAgent<C> {
    /// Refuse without both operator assertions, check the login, then
    /// ask with the prefix and body on stdin. A reply that does not pass
    /// [`parse`] or lacks measured usage is `Permanent`.
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
        if !self.lens_no_overage {
            return Err(unavailable("the no-overage assertion (lens_no_overage)"));
        }
        if !self.lens_host_cleared {
            return Err(unavailable(
                "the host-cleared assertion (lens_host_cleared)",
            ));
        }
        self.probe_subscription()?;
        let scratch = LensScratch::create().map_err(ClassifiedError::Transient)?;
        let prompt = format!("{}\n\n{}", q.prefix, q.body);
        let (exit, text) = self.exec(
            &self.lens_argv(&scratch.reply),
            prompt.as_bytes(),
            &scratch.reply,
        )?;
        let posts = parse(&text, &q.allowed)
            .map_err(|e| refuse(format!("{e}: {}", excerpt(text.as_bytes()))))?;
        let events = std::str::from_utf8(&exit.stdout)
            .map_err(|e| refuse(format!("events are not UTF-8: {e}")))?;
        let usage = usage(events).map_err(refuse)?;
        Ok(Answer { posts, usage })
    }
}

impl<C: CommandRunner> CodexAgent<C> {
    /// `codex login status`, accepted only for a ChatGPT login. Its output
    /// never reaches an error: an API-key login prints part of the key.
    fn probe_subscription(&self) -> Result<(), ClassifiedError> {
        let argv: Vec<String> = LOGIN_ARGV.iter().map(|a| (*a).to_owned()).collect();
        let exit = self
            .runner()
            .run_scrubbed(&argv, b"", LOGIN_TIMEOUT, SCRUBBED_ENV)
            .map_err(|e| ClassifiedError::Transient(AgentError(format!("codex: {e}"))))?;
        let said = |b: &[u8]| String::from_utf8_lossy(b).contains(SUBSCRIPTION_LOGIN);
        if exit.status == 0 && (said(&exit.stdout) || said(&exit.stderr)) {
            return Ok(());
        }
        Err(ClassifiedError::Unavailable(AgentError(
            "codex: lens questions need a ChatGPT subscription login (`codex login`)".to_owned(),
        )))
    }
}

fn unavailable(what: &str) -> ClassifiedError {
    ClassifiedError::Unavailable(AgentError(format!(
        "codex: lens questions need the operator's {what}"
    )))
}

fn refuse(e: String) -> ClassifiedError {
    ClassifiedError::Permanent(AgentError(format!("codex reply: {e}")))
}

/// The sum of the `turn.completed` usage events in `codex exec --json`
/// output, measured: at least one such event, each with integer
/// `input_tokens` and `output_tokens`, a `cached_input_tokens` (when
/// present) no greater than its input. Codex's input count includes the
/// cached part. Lines that are not JSON events are skipped.
pub(crate) fn usage(events: &str) -> Result<Usage, String> {
    let mut total: Option<Usage> = None;
    for v in events.lines().filter_map(|l| Value::parse(l.trim()).ok()) {
        if v.get("type").and_then(Value::as_str) != Some("turn.completed") {
            continue;
        }
        let turn = turn_usage(&v)?;
        let t = total.get_or_insert_with(Usage::default);
        t.input_tokens = t.input_tokens.saturating_add(turn.input_tokens);
        t.output_tokens = t.output_tokens.saturating_add(turn.output_tokens);
        t.cached_input_tokens = t
            .cached_input_tokens
            .saturating_add(turn.cached_input_tokens);
    }
    total.ok_or_else(|| "usage not reported (no turn.completed event)".to_owned())
}

fn turn_usage(v: &Value) -> Result<Usage, String> {
    let count = |k: &str, required: bool| -> Result<u64, String> {
        match v.pointer(&format!("/usage/{k}")) {
            Some(n) => n.as_u64().ok_or(format!("usage.{k} is not a count")),
            None if required => Err(format!("usage.{k} not reported")),
            None => Ok(0),
        }
    };
    let input = count("input_tokens", true)?;
    let cached = count("cached_input_tokens", false)?;
    if cached > input {
        return Err(format!(
            "usage reports {cached} cached of {input} input tokens"
        ));
    }
    Ok(Usage {
        input_tokens: input,
        output_tokens: count("output_tokens", true)?,
        cached_input_tokens: cached,
    })
}

/// A private scratch directory holding the last-message file, removed on
/// drop. The directory is created exclusively (an existing path of that
/// name, symlink or not, fails the call) and, on Unix, readable by this
/// user only, so nobody else can plant or swap the reply file inside it.
struct LensScratch {
    dir: PathBuf,
    reply: PathBuf,
}

impl LensScratch {
    fn create() -> Result<Self, AgentError> {
        let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let stem = format!("orch-codex-lens-{}-{seq}-{nanos}", std::process::id());
        Self::create_in(&std::env::temp_dir(), &stem)
    }

    fn create_in(base: &Path, stem: &str) -> Result<Self, AgentError> {
        let dir = base.join(stem);
        private_dir(&dir)
            .map_err(|e| AgentError(format!("codex: cannot create scratch dir: {e}")))?;
        Ok(Self {
            reply: dir.join("reply.txt"),
            dir,
        })
    }
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new().create(dir)
}

impl Drop for LensScratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_sums_completed_turns() {
        let events = concat!(
            "{\"type\":\"thread.started\"}\n",
            "not json\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":60,\"output_tokens\":7}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":3}}\n",
        );
        assert_eq!(
            usage(events),
            Ok(Usage {
                input_tokens: 110,
                output_tokens: 10,
                cached_input_tokens: 60,
            })
        );
    }

    #[test]
    fn unmeasured_usage_is_refused() {
        for events in [
            "",
            "{\"type\":\"thread.started\"}",
            "{\"type\":\"turn.completed\"}",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10}}",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":\"10\",\"output_tokens\":3}}",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"cached_input_tokens\":11,\"output_tokens\":3}}",
        ] {
            assert!(usage(events).is_err(), "{events}");
        }
    }

    #[test]
    fn a_scratch_name_already_taken_is_refused_and_left_alone() {
        let base =
            std::env::temp_dir().join(format!("orch-codex-scratch-test-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let victim = base.join("victim.txt");
        fs::write(&victim, "keep").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&base, base.join("taken")).unwrap();
        #[cfg(not(unix))]
        fs::create_dir(base.join("taken")).unwrap();

        assert!(LensScratch::create_in(&base, "taken").is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");

        let fresh = LensScratch::create_in(&base, "fresh").unwrap();
        assert_eq!(fresh.reply.parent(), Some(fresh.dir.as_path()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&fresh.dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let dir = fresh.dir.clone();
        drop(fresh);
        assert!(!dir.exists());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
        fs::remove_dir_all(&base).unwrap();
    }
}
