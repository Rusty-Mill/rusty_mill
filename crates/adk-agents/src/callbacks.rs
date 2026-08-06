//! Agent, model, and tool callbacks.
//!
//! ADK 2.0 makes callbacks the supported way to customize execution — custom
//! overrides of the 1.x `_run_async_impl` are no longer the right hook. Every
//! callback here follows the same contract: returning `Some(..)` **replaces**
//! the step it wraps, and returning `None` lets it proceed.

use adk_core::{Args, Content, InvocationContext};
use adk_models::{LlmRequest, LlmResponse};
use futures::future::BoxFuture;
use std::sync::Arc;

/// What a callback sees: the invocation, plus the agent it wraps.
#[derive(Clone)]
pub struct CallbackContext {
    /// The enclosing invocation.
    pub invocation: InvocationContext,
    /// The agent this callback is attached to.
    pub agent_name: String,
}

impl CallbackContext {
    /// Builds a callback context.
    pub fn new(invocation: InvocationContext, agent_name: impl Into<String>) -> Self {
        Self {
            invocation,
            agent_name: agent_name.into(),
        }
    }

    /// Reads a state key.
    pub fn state(&self, key: &str) -> Option<serde_json::Value> {
        self.invocation.get_state(key)
    }

    /// Stages a state write.
    pub fn set_state(&self, key: impl Into<String>, value: impl Into<serde_json::Value>) {
        self.invocation.set_state(key, value);
    }
}

impl std::fmt::Debug for CallbackContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackContext")
            .field("agent", &self.agent_name)
            .field("invocation_id", &self.invocation.invocation_id)
            .finish_non_exhaustive()
    }
}

/// Runs before the agent does any work.
///
/// Returning `Some(content)` skips the agent entirely and uses that content as
/// its response — the hook for guardrails and cache hits.
pub type BeforeAgentCallback =
    Arc<dyn for<'a> Fn(&'a CallbackContext) -> BoxFuture<'a, Option<Content>> + Send + Sync>;

/// Runs after the agent finishes.
///
/// Returning `Some(content)` replaces the agent's response.
pub type AfterAgentCallback =
    Arc<dyn for<'a> Fn(&'a CallbackContext) -> BoxFuture<'a, Option<Content>> + Send + Sync>;

/// Runs just before a model request is sent.
///
/// The request is mutable, so a callback can inject few-shot examples or
/// rewrite the system instruction. Returning `Some(response)` skips the model
/// call and uses that response.
pub type BeforeModelCallback = Arc<
    dyn for<'a> Fn(&'a CallbackContext, &'a mut LlmRequest) -> BoxFuture<'a, Option<LlmResponse>>
        + Send
        + Sync,
>;

/// Runs after a model response arrives.
///
/// Returning `Some(response)` replaces it — the hook for filtering or redaction.
pub type AfterModelCallback = Arc<
    dyn for<'a> Fn(&'a CallbackContext, &'a LlmResponse) -> BoxFuture<'a, Option<LlmResponse>>
        + Send
        + Sync,
>;

/// Runs before a tool executes.
///
/// Returning `Some(value)` skips the tool and uses that value as its result.
pub type BeforeToolCallback = Arc<
    dyn for<'a> Fn(&'a CallbackContext, &'a str, &'a Args) -> BoxFuture<'a, Option<serde_json::Value>>
        + Send
        + Sync,
>;

/// Runs after a tool executes.
///
/// Returning `Some(value)` replaces the tool's result.
pub type AfterToolCallback = Arc<
    dyn for<'a> Fn(
            &'a CallbackContext,
            &'a str,
            &'a Args,
            &'a serde_json::Value,
        ) -> BoxFuture<'a, Option<serde_json::Value>>
        + Send
        + Sync,
>;

/// The callbacks attached to an agent.
#[derive(Default, Clone)]
pub struct Callbacks {
    /// Runs before the agent starts.
    pub before_agent: Option<BeforeAgentCallback>,
    /// Runs after the agent finishes.
    pub after_agent: Option<AfterAgentCallback>,
    /// Runs before each model request.
    pub before_model: Option<BeforeModelCallback>,
    /// Runs after each model response.
    pub after_model: Option<AfterModelCallback>,
    /// Runs before each tool call.
    pub before_tool: Option<BeforeToolCallback>,
    /// Runs after each tool call.
    pub after_tool: Option<AfterToolCallback>,
}

impl Callbacks {
    /// An empty callback set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the before-agent callback.
    pub fn before_agent<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a CallbackContext) -> BoxFuture<'a, Option<Content>> + Send + Sync + 'static,
    {
        self.before_agent = Some(Arc::new(f));
        self
    }

    /// Sets the after-agent callback.
    pub fn after_agent<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a CallbackContext) -> BoxFuture<'a, Option<Content>> + Send + Sync + 'static,
    {
        self.after_agent = Some(Arc::new(f));
        self
    }

    /// Sets the before-model callback.
    pub fn before_model<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a CallbackContext, &'a mut LlmRequest) -> BoxFuture<'a, Option<LlmResponse>>
            + Send
            + Sync
            + 'static,
    {
        self.before_model = Some(Arc::new(f));
        self
    }

    /// Sets the after-model callback.
    pub fn after_model<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a CallbackContext, &'a LlmResponse) -> BoxFuture<'a, Option<LlmResponse>>
            + Send
            + Sync
            + 'static,
    {
        self.after_model = Some(Arc::new(f));
        self
    }

    /// Sets the before-tool callback.
    pub fn before_tool<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a CallbackContext, &'a str, &'a Args) -> BoxFuture<'a, Option<serde_json::Value>>
            + Send
            + Sync
            + 'static,
    {
        self.before_tool = Some(Arc::new(f));
        self
    }

    /// Sets the after-tool callback.
    pub fn after_tool<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(
                &'a CallbackContext,
                &'a str,
                &'a Args,
                &'a serde_json::Value,
            ) -> BoxFuture<'a, Option<serde_json::Value>>
            + Send
            + Sync
            + 'static,
    {
        self.after_tool = Some(Arc::new(f));
        self
    }
}

impl std::fmt::Debug for Callbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Callbacks")
            .field("before_agent", &self.before_agent.is_some())
            .field("after_agent", &self.after_agent.is_some())
            .field("before_model", &self.before_model.is_some())
            .field("after_model", &self.after_model.is_some())
            .field("before_tool", &self.before_tool.is_some())
            .field("after_tool", &self.after_tool.is_some())
            .finish()
    }
}
