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

use crate::agent_cli::{codex_usage, CliAgent, CliConfig, Staging, EVENT_BYTES};
use crate::error::RuntimeError;
use crate::executor::ProcessExecutor;

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
        let args = self.config.codex_args(&staging.tree(), &reply);
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
        let (prompt_tokens, completion_tokens) = codex_usage(&outcome.stdout)?;
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

fn read_reply(reply: &Path) -> Result<String, RuntimeError> {
    std::fs::read_to_string(reply)
        .map_err(|e| RuntimeError::Model(format!("codex wrote no reply ({e})")))
}

#[cfg(test)]
mod tests {
    use rsi_core::Role;

    use super::*;

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
