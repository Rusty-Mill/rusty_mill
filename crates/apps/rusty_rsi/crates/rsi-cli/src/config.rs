//! Model configuration from the environment, one set of variables per role.
//!
//! | Variable | Meaning |
//! |---|---|
//! | `RSI_<ROLE>_MODEL` | the model id (required) |
//! | `RSI_<ROLE>_BASE_URL` | an OpenAI-compatible base URL; default a local Ollama |
//! | `RSI_<ROLE>_API_KEY` | a bearer token, if the endpoint needs one |
//!
//! `<ROLE>` is `INNER` (the agent under test) or `OUTER` (the proposer).
//! Secrets come only from the environment and are never written anywhere.

use std::time::Duration;

use rsi_core::ModelId;
use rsi_runtime::OpenAiModel;

/// The default endpoint: a local Ollama.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

/// The longest a single model call may take (budgeted calls end sooner).
const MODEL_TIMEOUT: Duration = Duration::from_secs(900);

/// Which model a command needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The inner agent's model.
    Inner,
    /// The outer proposer's model.
    Outer,
}

impl Role {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Inner => "RSI_INNER",
            Self::Outer => "RSI_OUTER",
        }
    }
}

/// The model for `role`, from `RSI_<ROLE>_*`.
///
/// # Errors
/// When `RSI_<ROLE>_MODEL` is unset, or the id or URL is invalid.
pub fn model_from_env(role: Role) -> Result<OpenAiModel, String> {
    let var = |name: &str| std::env::var(format!("{}_{name}", role.prefix())).ok();
    let id = var("MODEL").ok_or_else(|| format!("{}_MODEL is not set", role.prefix()))?;
    let base_url = var("BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    OpenAiModel::new(
        ModelId::parse(&id).map_err(|e| e.to_string())?,
        &base_url,
        var("API_KEY"),
        MODEL_TIMEOUT,
    )
    .map_err(|e| e.to_string())
}
