//! The lens runner's [`Model`] port over `claude -p` (#594).

use orch_cli::{excerpt, CommandRunner};
use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{parse, Answer, Model, Question, Usage};
use rusty_json::Value;

use crate::agent::{envelope, result, ClaudeAgent};

impl<C: CommandRunner> ClaudeAgent<C> {
    /// The argv for one lens question: the base argv with the lens tool
    /// set ([`ClaudeAgent::lens_read_only`], else none) and the question's
    /// prefix as the whole system prompt. No schema: the reply is checked
    /// by [`orch_lens::parse`]. Public so tests and docs can pin it.
    pub fn lens_argv(&self, prefix: &str) -> Vec<String> {
        let mut argv = self.base_argv(self.lens_tools);
        argv.push("--system-prompt".to_owned());
        argv.push(prefix.to_owned());
        argv
    }
}

impl<C: CommandRunner> Model for ClaudeAgent<C> {
    /// Probe the login, then ask with the body on stdin. A reply that does
    /// not pass [`parse`] is `Permanent`.
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
        self.probe_login()?;
        let exit = self.exec(&self.lens_argv(&q.prefix), q.body.as_bytes())?;
        let envelope = envelope(&exit)?;
        let text = result(&envelope);
        let posts = parse(text, &q.allowed).map_err(|e| {
            ClassifiedError::Permanent(AgentError(format!(
                "claude reply: {e}: {}",
                excerpt(text.as_bytes())
            )))
        })?;
        Ok(Answer {
            posts,
            usage: usage(&envelope),
        })
    }
}

/// The envelope's `usage`. Claude reports fresh input, cache writes and
/// cache reads separately; all three are input, and reads are the cached
/// part. A missing count reads as zero.
fn usage(envelope: &Value) -> Usage {
    let count = |k: &str| {
        envelope
            .get("usage")
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cached = count("cache_read_input_tokens");
    Usage {
        input_tokens: count("input_tokens")
            .saturating_add(count("cache_creation_input_tokens"))
            .saturating_add(cached),
        output_tokens: count("output_tokens"),
        cached_input_tokens: cached,
    }
}
