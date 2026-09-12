use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use skillopt_core::{ChatBackend, Message};
use tokio::io::AsyncWriteExt;

/// The exact markers `skillopt_core::prompts::executor_system_prompt` wraps
/// a skill's raw text in before handing it to a `ChatBackend`. Kept in sync
/// with that function by `unwraps_skillopt_cores_executor_system_prompt`
/// below, which asserts against the real function rather than a copy of its
/// format string.
const SKILL_MARKER_START: &str = "--- SKILL ---\n";
const SKILL_MARKER_END: &str = "\n--- END SKILL ---";

/// Recovers the raw skill text `executor_system_prompt` wraps before a
/// rollout. This backend needs the unwrapped text to write out as AISF's
/// `FACTORY_PROMPTS_DIR` override -- AISF's own agent prompt must not be
/// polluted with skillopt's generic "you are an agent completing a
/// task... respond with only your final answer, no preamble" framing,
/// which is actively wrong guidance for a multi-turn, tool-calling agent
/// that needs to call real tools before it can answer at all.
fn extract_skill_text(wrapped: &str) -> anyhow::Result<&str> {
    let start = wrapped.find(SKILL_MARKER_START).ok_or_else(|| {
        anyhow::anyhow!(
            "expected an executor system prompt wrapping a skill between \
             \"--- SKILL ---\" markers (see \
             skillopt_core::prompts::executor_system_prompt); got: {wrapped:?}"
        )
    })? + SKILL_MARKER_START.len();
    let end = wrapped[start..].find(SKILL_MARKER_END).ok_or_else(|| {
        anyhow::anyhow!("missing \"--- END SKILL ---\" marker in executor system prompt")
    })?;
    Ok(&wrapped[start..start + end])
}

/// Creates a fresh, randomly-named scratch directory under the OS temp
/// dir. Backed by `tempfile`'s atomic, exclusive-create path generation
/// rather than a name built from `std::process::id()` plus a counter: a
/// predictable path in a shared temp dir can be pre-planted as a symlink
/// by another local user before this call ever runs, and a plain
/// `std::fs::create_dir_all` would happily follow it. The returned
/// `TempDir` deletes its directory on drop -- a leftover temp directory is
/// clutter, not a correctness problem.
fn new_scratch_dir(stage: &str) -> std::io::Result<tempfile::TempDir> {
    let prefix = format!("skillopt-aisf-{stage}-");
    tempfile::Builder::new().prefix(&prefix).tempdir()
}

/// A `ChatBackend` that runs one AISF factory stage's agent to completion
/// -- a whole multi-turn tool-use loop, gated by AISF's own governance --
/// via AISF's `eval-stage` subcommand, treating that entire loop as a
/// single opaque "chat call" the way `ChatBackend::chat`'s signature
/// already allows: it says nothing about how many real API calls happen
/// between the messages going in and the string coming out.
///
/// Expects `messages[0]` (system) to be `executor_system_prompt(skill)` --
/// which is what `Engine::run_executor` always sends -- and `messages[1]`
/// (user) to be the scenario JSON AISF's `eval-stage` reads on stdin (an
/// `Example::input`, authored that way by `skillopt_envs::AisfTriageEnv`).
/// The returned string is `eval-stage`'s JSON output line, unparsed --
/// `Environment::score` is what interprets it, not this backend.
pub struct AisfStageBackend {
    binary_path: PathBuf,
    stage: String,
}

impl AisfStageBackend {
    pub fn new(binary_path: PathBuf, stage: String) -> Self {
        Self { binary_path, stage }
    }
}

#[async_trait]
impl ChatBackend for AisfStageBackend {
    fn name(&self) -> &str {
        &self.stage
    }

    async fn chat(&self, messages: &[Message]) -> anyhow::Result<String> {
        anyhow::ensure!(
            messages.len() >= 2,
            "AisfStageBackend expects a system + user message, got {}",
            messages.len()
        );
        let skill_text = extract_skill_text(&messages[0].content)?;

        let scratch = new_scratch_dir(&self.stage)?;
        std::fs::write(scratch.path().join(format!("{}.md", self.stage)), skill_text)?;

        let mut child = tokio::process::Command::new(&self.binary_path)
            .arg("eval-stage")
            .arg(&self.stage)
            .env("FACTORY_PROMPTS_DIR", scratch.path())
            // Harmless when a real ANTHROPIC_API_KEY is set in this
            // process's own environment -- AISF's eval-stage checks for
            // a key first and only consults this as a fallback -- but it
            // means a sandbox with a working `claude` CLI session and no
            // key (this crate's own development sandbox, for one) just
            // works without the caller having to remember to export it.
            .env("EVAL_STAGE_DRIVER", "claude_cli")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                anyhow::anyhow!("failed to spawn AISF binary {:?}: {e}", self.binary_path)
            })?;

        let mut stdin = child.stdin.take().expect("stdin was requested as piped");
        stdin.write_all(messages[1].content.as_bytes()).await?;
        drop(stdin); // close our end so eval-stage's stdin read sees EOF

        let output = child.wait_with_output().await?;
        anyhow::ensure!(
            output.status.success(),
            "AISF `eval-stage {}` exited with {}: {}",
            self.stage,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skillopt_core::{prompts::executor_system_prompt, Skill};

    #[test]
    fn scratch_dir_path_is_not_predictable_from_pid_and_counter() {
        let d1 = new_scratch_dir("triage").unwrap();
        let d2 = new_scratch_dir("triage").unwrap();
        assert_ne!(d1.path(), d2.path());

        // The pre-fix implementation built the whole path from just
        // `std::process::id()` plus an incrementing counter, so any local
        // user could pre-plant a symlink at that exact path before this
        // call ever ran. None of the paths that scheme could have produced
        // may match the real, randomized path.
        let pid = std::process::id();
        let tmp = std::env::temp_dir();
        for n in 0..1000u64 {
            let guessed = tmp.join(format!("skillopt-aisf-triage-{pid}-{n}"));
            assert_ne!(d1.path(), guessed);
            assert_ne!(d2.path(), guessed);
        }
    }

    #[test]
    fn unwraps_skillopt_cores_executor_system_prompt() {
        let skill = Skill::new("# Triage\n- Classify by user-facing impact.\n");
        let wrapped = executor_system_prompt(&skill);
        assert_eq!(extract_skill_text(&wrapped).unwrap(), skill.text);
    }

    #[test]
    fn rejects_text_missing_the_start_marker() {
        assert!(extract_skill_text("no markers here").is_err());
    }

    #[test]
    fn rejects_text_missing_the_end_marker() {
        assert!(extract_skill_text("--- SKILL ---\nunterminated").is_err());
    }

    #[test]
    fn preserves_multiline_skill_content() {
        let skill = Skill::new("line one\nline two\nline three");
        let wrapped = executor_system_prompt(&skill);
        assert_eq!(extract_skill_text(&wrapped).unwrap(), skill.text);
    }

    #[tokio::test]
    async fn chat_rejects_a_single_message() {
        let backend = AisfStageBackend::new(PathBuf::from("/nonexistent/binary"), "triage".into());
        let err = backend.chat(&[Message::user("hi")]).await.unwrap_err();
        assert!(err.to_string().contains("expects a system + user message"));
    }

    #[tokio::test]
    async fn chat_rejects_an_unwrappable_system_message() {
        let backend = AisfStageBackend::new(PathBuf::from("/nonexistent/binary"), "triage".into());
        let err = backend
            .chat(&[Message::system("not wrapped"), Message::user("hi")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("SKILL"));
    }

    #[tokio::test]
    async fn chat_reports_a_clear_error_when_the_binary_is_missing() {
        let backend = AisfStageBackend::new(PathBuf::from("/nonexistent/binary"), "triage".into());
        let skill = Skill::new("# Triage\n");
        let messages = [
            Message::system(executor_system_prompt(&skill)),
            Message::user("{}"),
        ];
        let err = backend.chat(&messages).await.unwrap_err();
        assert!(err.to_string().contains("failed to spawn AISF binary"));
    }
}
