//! [`CodexModel`]: the Codex CLI as the inner agent's model (ADR-0005 §8).
//!
//! The inner agent (a0) stays the thing the outer loop improves, and the
//! broker still meters and records every call: only the completion comes
//! from `codex exec` instead of an OpenAI-compatible endpoint. Each call
//! runs one `codex exec --json` in a fresh, empty staging directory under
//! the same sandbox as [`crate::agent_cli`], so Codex can reach its model
//! but not the tasks, the repository or the run.
//!
//! - **Tokens.** Read from the `turn.completed` event's `usage`; a run
//!   that reports none is an error, never an unmetered call. Codex has no
//!   per-call output cap, so one call may overrun the budget's remaining
//!   tokens; the broker charges it and refuses the next call.
//! - **Time.** The call is killed at the timeout the broker passes (the
//!   budget's remaining time).
//! - **Login.** Codex's own (`codex login`); no key passes through `rsi`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::{ChatModel, Completion, Message, ModelId};
use rusty_json::Value;

use crate::agent_cli::{CliAgent, CliConfig, Staging};
use crate::error::RuntimeError;
use crate::executor::ProcessExecutor;

/// Bytes of Codex's event stream kept: enough for a long turn, whose
/// `turn.completed` event comes last.
const EVENT_BYTES: usize = 8 << 20;

const CONTRACT: &str = "You are the language model behind an automated \
agent. Reply to the conversation below as the assistant: text only. Do not \
run commands, read or edit files, or browse; everything you need is in the \
conversation.";

/// Completions from `codex exec`, confined by the sandbox.
#[derive(Debug, Clone)]
pub struct CodexModel {
    executor: ProcessExecutor,
    config: CliConfig,
    scratch: PathBuf,
    protected: Vec<PathBuf>,
    id: ModelId,
}

impl CodexModel {
    /// A model that runs each call in a staging directory under `scratch`
    /// and refuses to run Codex if its sandbox could reach any `protected`
    /// path.
    ///
    /// # Errors
    /// [`RuntimeError::Model`] if `config` is not for Codex, a path is
    /// relative, or the model name is not a valid id.
    pub fn new(
        executor: &ProcessExecutor,
        config: CliConfig,
        scratch: PathBuf,
        protected: Vec<PathBuf>,
    ) -> Result<Self, RuntimeError> {
        if config.agent != CliAgent::Codex {
            return Err(RuntimeError::Model(format!(
                "{} cannot serve as a chat model; only codex can",
                config.agent.name()
            )));
        }
        config.check_paths(&scratch)?;
        let id = config.model_id()?;
        Ok(Self {
            executor: executor.clone().with_capture_bytes(EVENT_BYTES),
            config,
            scratch,
            protected,
            id,
        })
    }
}

impl ChatModel for CodexModel {
    type Error = RuntimeError;

    fn id(&self) -> &ModelId {
        &self.id
    }

    fn complete(
        &self,
        messages: &[Message],
        _max_tokens: u64,
        timeout: Duration,
    ) -> Result<Completion, RuntimeError> {
        if timeout.is_zero() {
            return Err(RuntimeError::Model("no time left for a codex call".into()));
        }
        let staging = Staging::create(&self.scratch, CliAgent::Codex)?;
        let prompt = staging.write_prompt(&render(messages))?;
        let reply = staging.tmp().join("reply.md");
        let args = self.config.codex_args(&staging.tree(), &reply, true);
        let outcome = self.config.run(
            &self.executor,
            &staging,
            &args,
            &prompt,
            timeout,
            &self.protected,
        )?;
        if !outcome.termination.succeeded() {
            return Err(self.config.failure(&outcome));
        }
        let (prompt_tokens, completion_tokens) = usage(&outcome.stdout)?;
        let text = read_reply(&reply)?;
        Ok(Completion {
            text,
            prompt_tokens,
            completion_tokens,
        })
    }
}

/// The conversation as one prompt: the contract, then each message under
/// its role.
fn render(messages: &[Message]) -> String {
    let mut out = format!("{CONTRACT}\n");
    for message in messages {
        out.push_str(&format!(
            "\n## {}\n\n{}\n",
            message.role.as_str(),
            message.content
        ));
    }
    out.push_str("\n## assistant\n");
    out
}

/// Prompt and completion tokens from the `turn.completed` events in
/// Codex's JSONL output, summed.
///
/// # Errors
/// [`RuntimeError::Model`] for a `turn.failed` event, or when no turn
/// reported its usage.
fn usage(stdout: &[u8]) -> Result<(u64, u64), RuntimeError> {
    let text = String::from_utf8_lossy(stdout);
    let mut total: Option<(u64, u64)> = None;
    for line in text.lines().map(str::trim).filter(|l| l.starts_with('{')) {
        let Ok(event) = Value::parse(line) else {
            continue;
        };
        match event.pointer("/type").and_then(Value::as_str) {
            Some("turn.failed") => {
                let message = event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("no message");
                return Err(RuntimeError::Model(format!("codex turn failed: {message}")));
            }
            Some("turn.completed") => {
                let count = |key: &str| {
                    event
                        .pointer(&format!("/usage/{key}"))
                        .and_then(Value::as_u64)
                };
                let (Some(input), Some(output)) = (count("input_tokens"), count("output_tokens"))
                else {
                    return Err(RuntimeError::Model(
                        "codex's turn.completed event has no usage".into(),
                    ));
                };
                let (p, c) = total.unwrap_or((0, 0));
                total = Some((p.saturating_add(input), c.saturating_add(output)));
            }
            _ => {}
        }
    }
    total.ok_or_else(|| {
        RuntimeError::Model("codex reported no token usage, so the call cannot be metered".into())
    })
}

fn read_reply(reply: &Path) -> Result<String, RuntimeError> {
    std::fs::read_to_string(reply)
        .map_err(|e| RuntimeError::Model(format!("codex wrote no reply ({e})")))
}

#[cfg(test)]
mod tests {
    use rsi_core::Role;

    use super::*;

    #[test]
    fn usage_comes_from_turn_completed_and_is_required() {
        let events = b"{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n\
{\"type\":\"turn.started\"}\n\
{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"hi\"}}\n\
{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":120,\"cached_input_tokens\":100,\"output_tokens\":7}}\n";
        assert_eq!(usage(events).expect("usage"), (120, 7));
        let e = usage(b"{\"type\":\"turn.started\"}\n").expect_err("no usage");
        assert!(e.to_string().contains("cannot be metered"), "{e}");
        let e = usage(b"{\"type\":\"turn.failed\",\"error\":{\"message\":\"quota\"}}\n")
            .expect_err("failed");
        assert!(e.to_string().contains("quota"), "{e}");
        let e = usage(b"{\"type\":\"turn.completed\",\"usage\":{}}\n").expect_err("empty");
        assert!(e.to_string().contains("no usage"), "{e}");
    }

    #[test]
    fn the_prompt_keeps_every_message_under_its_role() {
        let prompt = render(&[
            Message {
                role: Role::System,
                content: "be brief".into(),
            },
            Message {
                role: Role::User,
                content: "solve it".into(),
            },
        ]);
        assert!(prompt.starts_with(CONTRACT), "{prompt}");
        let system = prompt.find("## system\n\nbe brief").expect("system");
        let user = prompt.find("## user\n\nsolve it").expect("user");
        assert!(
            system < user && prompt.ends_with("## assistant\n"),
            "{prompt}"
        );
    }

    #[test]
    fn only_codex_can_be_a_chat_model() {
        let executor = ProcessExecutor::new("/rsi".into(), vec![], "/tmp/state".into());
        let config = CliConfig {
            agent: CliAgent::Claude,
            program: "/opt/claude/bin/claude".into(),
            model: None,
            home: "/home/u/.claude".into(),
            wall: Duration::from_secs(60),
            env: vec![],
        };
        let e = CodexModel::new(&executor, config, "/tmp/s".into(), vec![]).expect_err("claude");
        assert!(e.to_string().contains("only codex"), "{e}");
    }
}
