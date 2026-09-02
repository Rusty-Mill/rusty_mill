use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use skillopt_core::{ChatBackend, Message, Role};
use tokio::io::AsyncWriteExt;

/// Deletes its file on drop. Best-effort, mirroring `aisf_stage::ScratchDir`
/// -- a leftover temp file is clutter, not a correctness problem.
struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_scratch_system_prompt_file(content: &str) -> std::io::Result<ScratchFile> {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "skillopt-claude-cli-system-{}-{n}.txt",
        std::process::id()
    ));
    std::fs::write(&path, content)?;
    Ok(ScratchFile(path))
}

/// Splits a `ChatBackend::chat` call's messages into (system_prompt,
/// user_prompt) -- `claude -p` takes one system prompt and one turn of
/// input, not an arbitrary message list, but every existing call site in
/// `skillopt-core::Engine` only ever sends at most one system message plus
/// one user message per call (`run_executor` sends both; `optimize`/
/// `reflect` send only a user message). Concatenating same-role messages
/// with blank lines (rather than hard-erroring on more than one of either)
/// costs nothing and degrades gracefully if that ever changes.
fn partition_messages(messages: &[Message]) -> anyhow::Result<(String, String)> {
    anyhow::ensure!(!messages.is_empty(), "ClaudeCliBackend got no messages");

    let system_prompt = messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let user_prompt = messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    anyhow::ensure!(
        !user_prompt.is_empty(),
        "ClaudeCliBackend got only system message(s), no user content to send"
    );
    Ok((system_prompt, user_prompt))
}

/// A `ChatBackend` that shells out to the `claude` CLI's non-interactive
/// print mode (`claude -p`) instead of calling the Anthropic API directly.
/// Useful wherever a working `claude` CLI session exists (e.g. an
/// OAuth-authenticated Claude Code sandbox) but no portable
/// `ANTHROPIC_API_KEY` is available for a raw HTTP client -- confirmed in
/// this project's own development sandbox: raw HTTPS to
/// `api.anthropic.com` 401s with no key, while `claude -p` already has a
/// working session.
///
/// All built-in tools are disabled (`--tools ""`) and sessions aren't
/// persisted (`--no-session-persistence`): this is a plain single-turn
/// text completion, not an invitation for the CLI's own agentic loop to
/// read/write files or run commands mid-training-run. That also means it
/// cannot stand in for `aisf_stage`'s executor role, which genuinely needs
/// a governed tool-use loop -- this backend is for the optimizer/reflector
/// roles (or any other plain chat completion), not a replacement for
/// `AisfStageBackend`.
pub struct ClaudeCliBackend {
    model: String,
}

impl ClaudeCliBackend {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

#[async_trait]
impl ChatBackend for ClaudeCliBackend {
    fn name(&self) -> &str {
        "claude_cli"
    }

    async fn chat(&self, messages: &[Message]) -> anyhow::Result<String> {
        let (system_prompt, user_prompt) = partition_messages(messages)?;

        let mut cmd = tokio::process::Command::new("claude");
        cmd.arg("-p")
            .arg("--tools")
            .arg("")
            .arg("--no-session-persistence")
            .arg("--output-format")
            .arg("text")
            .arg("--model")
            .arg(&self.model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let _sys_file = if system_prompt.is_empty() {
            None
        } else {
            let f = new_scratch_system_prompt_file(&system_prompt)?;
            cmd.arg("--system-prompt-file").arg(&f.0);
            Some(f)
        };

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn `claude` CLI (is it on PATH?): {e}"))?;

        let mut stdin = child.stdin.take().expect("stdin was requested as piped");
        stdin.write_all(user_prompt.as_bytes()).await?;
        drop(stdin); // close our end so claude -p's stdin read sees EOF

        let output = child.wait_with_output().await?;
        anyhow::ensure!(
            output.status.success(),
            "`claude -p` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_system_and_user_messages() {
        let messages = [Message::system("be terse"), Message::user("say hi")];
        let (system, user) = partition_messages(&messages).unwrap();
        assert_eq!(system, "be terse");
        assert_eq!(user, "say hi");
    }

    #[test]
    fn user_only_messages_have_an_empty_system_prompt() {
        let messages = [Message::user("propose an edit")];
        let (system, user) = partition_messages(&messages).unwrap();
        assert_eq!(system, "");
        assert_eq!(user, "propose an edit");
    }

    #[test]
    fn multiple_user_messages_are_joined() {
        let messages = [Message::user("first"), Message::user("second")];
        let (_, user) = partition_messages(&messages).unwrap();
        assert_eq!(user, "first\n\nsecond");
    }

    #[test]
    fn empty_message_list_errors() {
        assert!(partition_messages(&[]).is_err());
    }

    #[test]
    fn system_only_messages_error() {
        let messages = [Message::system("be terse")];
        assert!(partition_messages(&messages).is_err());
    }

    #[tokio::test]
    async fn chat_reports_a_clear_error_when_claude_is_missing_from_path() {
        // Overriding PATH to something with no `claude` binary is the
        // simplest deterministic way to exercise the spawn-failure path
        // without depending on whether this machine happens to have the
        // CLI installed. Restored afterward since PATH is process-global
        // and Rust runs tests in the same process concurrently by default.
        let original_path = std::env::var_os("PATH");
        std::env::set_var("PATH", "/nonexistent");
        let backend = ClaudeCliBackend::new("sonnet".to_string());
        let err = backend.chat(&[Message::user("hi")]).await.unwrap_err();
        if let Some(path) = original_path {
            std::env::set_var("PATH", path);
        }
        assert!(err.to_string().contains("failed to spawn `claude` CLI"));
    }
}
