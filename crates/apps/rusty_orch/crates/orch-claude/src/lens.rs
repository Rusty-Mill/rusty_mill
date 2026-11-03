//! The lens runner's [`Model`] port over `claude -p` (#594).
//!
//! `claude -p` has no before-call bound on an agent loop's tokens, so this
//! adapter is admitted only at zero price, and only where no call can add a
//! charge. Two separate gates, both fail closed: the operator's assertion
//! that one named account cannot bill beyond its plan
//! ([`ClaudeAgent::lens_no_overage`]), and, on every question, a
//! subscription login to that same account (`email` in `claude auth
//! status`). The login check establishes the auth method and the account,
//! not billing: a subscription with usage credits enabled can still be
//! charged, which is why the first gate exists.

use orch_cli::{excerpt, CommandRunner, Exit};
use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{parse, Answer, Model, Question, Usage};
use rusty_json::Value;

use crate::agent::{envelope, result, ClaudeAgent};

/// `authMethod` values of `claude auth status` that bill the Claude
/// subscription. `oauth_token` is `CLAUDE_CODE_OAUTH_TOKEN`, the
/// subscription credential in headless setups (ADR-0012). Every other
/// value, including an unknown one, is refused.
pub const SUBSCRIPTION_AUTH: &[&str] = &["claude.ai", "oauth_token"];

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

    /// The login probe, accepted only for a subscription login to
    /// `account`. The reported account never reaches the error.
    fn probe_subscription(&self, account: &str) -> Result<(), ClassifiedError> {
        let exit = self.probe()?;
        let status = std::str::from_utf8(&exit.stdout)
            .ok()
            .and_then(|s| Value::parse(s.trim()).ok());
        let field = |k: &str| {
            status
                .as_ref()
                .and_then(|v| v.get(k))
                .and_then(Value::as_str)
                .map(str::trim)
        };
        let method = field("authMethod");
        if !method.is_some_and(|m| SUBSCRIPTION_AUTH.contains(&m)) {
            return Err(unavailable(format!(
                "claude: lens questions need a subscription login, got authMethod {}",
                method.unwrap_or("(unreported)")
            )));
        }
        match field("email") {
            Some(e) if e.eq_ignore_ascii_case(account.trim()) => Ok(()),
            Some(_) => Err(unavailable(
                "claude: logged into another account than the no-overage assertion names".to_owned(),
            )),
            None => Err(unavailable(
                "claude: the login does not report its account, so the no-overage assertion cannot apply"
                    .to_owned(),
            )),
        }
    }
}

impl<C: CommandRunner> Model for ClaudeAgent<C> {
    /// Refuse without the no-overage assertion, check the subscription
    /// login and its account, then ask with the body on stdin. A
    /// reply that is not UTF-8, does not pass [`parse`], or does not report
    /// its usage is `Permanent`.
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
        let Some(account) = self.lens_no_overage.as_deref() else {
            return Err(unavailable(
                "claude: lens questions need the operator's no-overage assertion (lens_no_overage)"
                    .to_owned(),
            ));
        };
        self.probe_subscription(account)?;
        let exit = self.exec(&self.lens_argv(&q.prefix), q.body.as_bytes())?;
        strict_utf8(&exit)?;
        let envelope = envelope(&exit)?;
        let usage = usage(&envelope).map_err(refuse)?;
        let text = result(&envelope);
        let posts = parse(text, &q.allowed)
            .map_err(|e| refuse(format!("{e}: {}", excerpt(text.as_bytes()))))?;
        Ok(Answer { posts, usage })
    }
}

fn refuse(e: String) -> ClassifiedError {
    ClassifiedError::Permanent(AgentError(format!("claude reply: {e}")))
}

/// The envelope is decoded leniently for error text; a lens reply must be
/// exact, so a byte that is not UTF-8 refuses the whole reply.
fn strict_utf8(exit: &Exit) -> Result<(), ClassifiedError> {
    if exit.status != 0 {
        return Ok(());
    }
    std::str::from_utf8(&exit.stdout)
        .map(drop)
        .map_err(|e| refuse(format!("not UTF-8: {e}")))
}

/// The envelope's `usage`, measured. Claude reports fresh input, cache
/// writes and cache reads separately; all three are input, and reads are
/// the cached part. `input_tokens` and `output_tokens` are required; the
/// cache counts, when present, must be integers too.
fn usage(envelope: &Value) -> Result<Usage, String> {
    let u = envelope.get("usage").ok_or("usage not reported")?;
    let count = |k: &str, required: bool| -> Result<u64, String> {
        match u.get(k) {
            Some(v) => v.as_u64().ok_or(format!("usage.{k} is not a count")),
            None if required => Err(format!("usage.{k} not reported")),
            None => Ok(0),
        }
    };
    let cached = count("cache_read_input_tokens", false)?;
    Ok(Usage {
        input_tokens: count("input_tokens", true)?
            .saturating_add(count("cache_creation_input_tokens", false)?)
            .saturating_add(cached),
        output_tokens: count("output_tokens", true)?,
        cached_input_tokens: cached,
    })
}

fn unavailable(message: String) -> ClassifiedError {
    ClassifiedError::Unavailable(AgentError(message))
}
