//! The lens runner's [`Model`] port over `codex exec` (#594).
//!
//! Admitted only as a flat-rate subscription provider: the lens config
//! refuses any nonzero price, and every question first proves the ChatGPT
//! login. `codex exec` has no before-call bound on its agent loop.

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
    /// The argv for one lens question run in `cwd`: the base argv, JSONL
    /// events on stdout (for usage), the last message to `reply`, the
    /// prompt on stdin. No schema: the reply is checked by
    /// [`orch_lens::parse`]. Public so tests and docs can pin it.
    pub fn lens_argv(&self, cwd: &Path, reply: &Path) -> Vec<String> {
        let mut argv = self.base_argv(cwd);
        // The scratch directory is not a git repository.
        argv.push("--skip-git-repo-check".to_owned());
        argv.push("--json".to_owned());
        argv.push("--output-last-message".to_owned());
        argv.push(reply.display().to_string());
        argv.push("-".to_owned());
        argv
    }
}

impl<C: CommandRunner> Model for CodexAgent<C> {
    /// Ask with the prefix and body on stdin, in the repository root for a
    /// read-only lens and an empty scratch directory otherwise. A reply
    /// that does not pass [`parse`] is `Permanent`.
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
        self.probe_subscription()?;
        let scratch = LensScratch::create().map_err(ClassifiedError::Transient)?;
        let cwd = if self.lens_reads_repo {
            self.repo_root().to_path_buf()
        } else {
            scratch.dir.clone()
        };
        let prompt = format!("{}\n\n{}", q.prefix, q.body);
        let (exit, text) = self.exec(
            &self.lens_argv(&cwd, &scratch.reply),
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

/// An empty working directory and a last-message file beside it, both in
/// the OS temp dir and removed on drop. One pair per question.
struct LensScratch {
    dir: PathBuf,
    reply: PathBuf,
}

impl LensScratch {
    fn create() -> Result<Self, AgentError> {
        let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
        let stem = format!("orch-codex-lens-{}-{seq}", std::process::id());
        let tmp = std::env::temp_dir();
        let dir = tmp.join(&stem);
        fs::create_dir(&dir)
            .map_err(|e| AgentError(format!("codex: cannot create scratch dir: {e}")))?;
        Ok(Self {
            dir,
            reply: tmp.join(format!("{stem}.reply.txt")),
        })
    }
}

impl Drop for LensScratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
        let _ = fs::remove_file(&self.reply);
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
}
