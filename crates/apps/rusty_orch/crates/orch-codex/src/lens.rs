//! The lens runner's [`Model`] port over `codex exec` (#594).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use orch_cli::{excerpt, CommandRunner};
use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{parse, Answer, Model, Question, Usage};
use rusty_json::Value;

use crate::agent::{CodexAgent, RUN_SEQ};

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
        let posts = parse(&text, &q.allowed).map_err(|e| {
            ClassifiedError::Permanent(AgentError(format!(
                "codex reply: {e}: {}",
                excerpt(text.as_bytes())
            )))
        })?;
        Ok(Answer {
            posts,
            usage: usage(&String::from_utf8_lossy(&exit.stdout)),
        })
    }
}

/// The sum of the `turn.completed` usage events in `codex exec --json`
/// output. Codex's input count includes the cached part; a cached count
/// above it is held to it. Lines that are not JSON are skipped.
pub(crate) fn usage(events: &str) -> Usage {
    let mut total = Usage::default();
    for v in events.lines().filter_map(|l| Value::parse(l.trim()).ok()) {
        if v.get("type").and_then(Value::as_str) != Some("turn.completed") {
            continue;
        }
        let count = |k: &str| {
            v.pointer(&format!("/usage/{k}"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        total.input_tokens = total.input_tokens.saturating_add(count("input_tokens"));
        total.output_tokens = total.output_tokens.saturating_add(count("output_tokens"));
        total.cached_input_tokens = total
            .cached_input_tokens
            .saturating_add(count("cached_input_tokens"));
    }
    total.cached_input_tokens = total.cached_input_tokens.min(total.input_tokens);
    total
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
    fn usage_sums_completed_turns_and_bounds_the_cached_part() {
        let events = concat!(
            "{\"type\":\"thread.started\"}\n",
            "not json\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":60,\"output_tokens\":7}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"cached_input_tokens\":90,\"output_tokens\":3}}\n",
        );
        assert_eq!(
            usage(events),
            Usage {
                input_tokens: 110,
                output_tokens: 10,
                cached_input_tokens: 110,
            }
        );
        assert_eq!(usage(""), Usage::default());
    }
}
