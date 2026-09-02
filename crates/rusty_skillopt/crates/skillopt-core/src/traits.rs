use async_trait::async_trait;

use crate::types::{Example, Message};

/// A chat-completion backend. Implemented once per LLM provider in
/// `skillopt-model`; the engine only ever talks to `dyn ChatBackend`.
#[async_trait]
pub trait ChatBackend: Send + Sync {
    /// Human-readable identifier, used in logs and the training report.
    fn name(&self) -> &str;

    async fn chat(&self, messages: &[Message]) -> anyhow::Result<String>;
}

/// A benchmark/task adapter. Implemented once per benchmark in
/// `skillopt-envs`.
pub trait Environment: Send + Sync {
    fn name(&self) -> &str;

    fn train_examples(&self) -> &[Example];
    fn val_examples(&self) -> &[Example];
    fn test_examples(&self) -> &[Example];

    /// Programmatic, deterministic reward in `[0, 1]` for a given output.
    /// Kept separate from the (LLM-based) reflection critique so the
    /// validation gate never depends on a model call.
    fn score(&self, example: &Example, output: &str) -> f64;
}
