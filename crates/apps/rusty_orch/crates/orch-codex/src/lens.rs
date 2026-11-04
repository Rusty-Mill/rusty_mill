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

use std::time::Duration;

use orch_cli::{excerpt, CommandRunner};
use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{check, Answer, Model, Question, Usage};
use rusty_json::Value;

use crate::agent::{CodexAgent, SCRUBBED_ENV};

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
    /// root, JSONL events on stdout (the reply and usage both come from
    /// them, so nothing is written to disk and the run can happen in a
    /// container), the prompt on stdin. No schema: the reply is checked by
    /// [`orch_lens::parse`]. Public so tests and docs can pin it.
    pub fn lens_argv(&self) -> Vec<String> {
        let mut argv = self.base_argv(self.repo_root());
        argv.push("--json".to_owned());
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
        let prompt = format!("{}\n\n{}", q.prefix, q.body);
        let exit = self.run_checked(&self.lens_argv(), prompt.as_bytes())?;
        let events = std::str::from_utf8(&exit.stdout)
            .map_err(|e| refuse(format!("events are not UTF-8: {e}")))?;
        let text = last_message(events).map_err(|e| {
            ClassifiedError::Transient(AgentError(format!("codex: {e}: {}", excerpt(&exit.stderr))))
        })?;
        let reply = check(&text, &q.shape)
            .map_err(|e| refuse(format!("{e}: {}", excerpt(text.as_bytes()))))?;
        let usage = usage(events).map_err(refuse)?;
        Ok(Answer { reply, usage })
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

/// The text of the last completed `agent_message` item in `codex exec
/// --json` output (`{"type":"item.completed","item":{"type":
/// "agent_message","text":...}}`), which is the run's final message.
/// Missing or blank is an error. Lines that are not JSON events are skipped.
pub(crate) fn last_message(events: &str) -> Result<String, String> {
    // The last completed message is the reply even when it is malformed:
    // never fall back to an earlier one.
    let last = events
        .lines()
        .rev()
        .filter_map(|l| Value::parse(l.trim()).ok())
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("item.completed"))
        .find(|v| v.pointer("/item/type").and_then(Value::as_str) == Some("agent_message"))
        .ok_or("codex reported no final message")?;
    let text = last
        .pointer("/item/text")
        .and_then(Value::as_str)
        .ok_or("codex's final message has no text")?
        .to_owned();
    if text.trim().is_empty() {
        return Err("codex reported an empty final message".to_owned());
    }
    Ok(text)
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
    fn the_last_agent_message_is_the_reply() {
        let events = concat!(
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"first\"}}\n",
            "not json\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"reasoning\",\"text\":\"thinking\"}}\n",
            "{\"type\":\"item.started\",\"item\":{\"type\":\"agent_message\",\"text\":\"partial\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"final\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"reasoning\",\"text\":\"after\"}}\n",
        );
        assert_eq!(last_message(events), Ok("final".to_owned()));
    }

    #[test]
    fn a_malformed_final_message_never_falls_back_to_an_earlier_one() {
        let earlier = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"earlier\"}}\n";
        for last in [
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\"}}",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":7}}",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\" \"}}",
        ] {
            let events = format!("{earlier}{last}\n");
            assert!(last_message(&events).is_err(), "{events}");
        }
    }

    #[test]
    fn no_or_a_blank_final_message_is_refused() {
        for events in [
            "",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"reasoning\",\"text\":\"x\"}}",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\"}}",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"  \"}}",
        ] {
            assert!(last_message(events).is_err(), "{events}");
        }
    }
}
