use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use skillopt_core::{ChatBackend, Message, Role};
use tokio::io::AsyncWriteExt;

/// Creates a fresh, randomly-named scratch file under the OS temp dir and
/// writes `content` to it. Backed by `tempfile`'s atomic, exclusive-create
/// path generation rather than a name built from `std::process::id()` plus
/// a counter: a predictable path in a shared temp dir can be pre-planted
/// as a symlink by another local user before this call ever runs, and a
/// plain `std::fs::write` would happily follow it. The returned
/// `NamedTempFile` deletes itself on drop, mirroring `aisf_stage`'s
/// `TempDir` usage -- a leftover temp file is clutter, not a correctness
/// problem.
fn new_scratch_system_prompt_file(content: &str) -> std::io::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("skillopt-claude-cli-system-")
        .suffix(".txt")
        .tempfile()?;
    file.write_all(content.as_bytes())?;
    Ok(file)
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
    timeout: Duration,
}

impl ClaudeCliBackend {
    pub fn new(model: String) -> Self {
        Self {
            model,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Overrides the default 300s subprocess wait timeout. A `claude -p`
    /// invocation that never exits (hung auth prompt, wedged CLI session,
    /// etc.) would otherwise block `chat()` -- and therefore
    /// `Engine::train` -- forever; see [`ClaudeCliError::Timeout`].
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Default `claude -p` subprocess wait timeout. Long enough for a real
/// completion (these may be long-running CLI invocations), short enough to
/// still recover a stuck `Engine::train` run instead of hanging forever.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Errors specific to [`ClaudeCliBackend`] that a caller may want to match
/// on directly rather than parse out of the `anyhow` chain's `Display` text.
#[derive(Debug, thiserror::Error)]
pub enum ClaudeCliError {
    #[error("`claude -p` did not finish within {0:?} and was killed")]
    Timeout(Duration),
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
            .stderr(Stdio::piped())
            // A timed-out wait below drops this future's `Child`; killing
            // the still-running process on drop (rather than orphaning it)
            // is exactly what makes the timeout actually recover the slot
            // instead of leaking a wedged subprocess.
            .kill_on_drop(true);

        let _sys_file = if system_prompt.is_empty() {
            None
        } else {
            let f = new_scratch_system_prompt_file(&system_prompt)?;
            cmd.arg("--system-prompt-file").arg(f.path());
            Some(f)
        };

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn `claude` CLI (is it on PATH?): {e}"))?;

        let mut stdin = child.stdin.take().expect("stdin was requested as piped");
        stdin.write_all(user_prompt.as_bytes()).await?;
        drop(stdin); // close our end so claude -p's stdin read sees EOF

        let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(result) => result?,
            Err(_elapsed) => return Err(ClaudeCliError::Timeout(self.timeout).into()),
        };
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
    fn scratch_system_prompt_file_path_is_not_predictable_from_pid_and_counter() {
        let f1 = new_scratch_system_prompt_file("one").unwrap();
        let f2 = new_scratch_system_prompt_file("two").unwrap();
        assert_ne!(f1.path(), f2.path());

        // The pre-fix implementation built the whole path from just
        // `std::process::id()` plus an incrementing counter, so any local
        // user could pre-plant a symlink at that exact path before this
        // call ever ran. None of the paths that scheme could have produced
        // may match the real, randomized path.
        let pid = std::process::id();
        let tmp = std::env::temp_dir();
        for n in 0..1000u64 {
            let guessed = tmp.join(format!("skillopt-claude-cli-system-{pid}-{n}.txt"));
            assert_ne!(f1.path(), guessed);
            assert_ne!(f2.path(), guessed);
        }
    }

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
