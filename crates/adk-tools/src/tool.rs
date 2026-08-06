//! The [`Tool`] trait and the confirmation policy that governs a call.

use adk_core::{AdkError, Args, FunctionDeclaration, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::context::ToolContext;

/// When a tool call needs explicit user approval before it runs.
pub enum ConfirmationPolicy {
    /// Never ask.
    Never,
    /// Always ask, with this prompt.
    Always(String),
    /// Ask only when the predicate says so, given the call's arguments.
    ///
    /// This is the conditional form ADK supports — approve small refunds
    /// silently, escalate large ones.
    Conditional(Box<dyn Fn(&Args) -> Option<String> + Send + Sync>),
}

impl ConfirmationPolicy {
    /// Returns the prompt to show, or `None` when no approval is needed.
    pub fn hint_for(&self, args: &Args) -> Option<String> {
        match self {
            ConfirmationPolicy::Never => None,
            ConfirmationPolicy::Always(hint) => Some(hint.clone()),
            ConfirmationPolicy::Conditional(predicate) => predicate(args),
        }
    }
}

impl Default for ConfirmationPolicy {
    fn default() -> Self {
        ConfirmationPolicy::Never
    }
}

impl std::fmt::Debug for ConfirmationPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfirmationPolicy::Never => f.write_str("Never"),
            ConfirmationPolicy::Always(h) => write!(f, "Always({h:?})"),
            ConfirmationPolicy::Conditional(_) => f.write_str("Conditional(..)"),
        }
    }
}

/// Something an agent can invoke.
///
/// Implement this directly for tools with custom dispatch; for ordinary Rust
/// functions prefer [`crate::FunctionTool`] or the `#[adk_tool]` macro, which
/// derive the declaration from the signature and doc comment.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The name the model uses to call this tool. Must be unique per agent.
    fn name(&self) -> &str;

    /// What the tool does and when to use it.
    fn description(&self) -> &str;

    /// The declaration sent to the model.
    ///
    /// Returning `None` hides the tool from the model — useful for tools
    /// invoked only by other code paths.
    fn declaration(&self) -> Option<FunctionDeclaration> {
        Some(FunctionDeclaration::new(self.name(), self.description()))
    }

    /// Whether this tool runs in the background rather than blocking the turn.
    ///
    /// A long-running tool returns an acknowledgement immediately; the agent
    /// continues, and the caller supplies the real result later. Its call id is
    /// recorded in [`adk_core::Event::long_running_tool_ids`].
    fn is_long_running(&self) -> bool {
        false
    }

    /// The approval prompt for a call with these arguments, or `None` to run
    /// without asking.
    ///
    /// Taking the arguments lets a tool gate conditionally — approve small
    /// refunds silently and escalate large ones — which is the behaviour
    /// [`ConfirmationPolicy::Conditional`] expresses.
    fn confirmation_hint(&self, args: &Args) -> Option<String> {
        let _ = args;
        None
    }

    /// Runs the tool.
    ///
    /// The returned value is normalized to ADK's object convention by
    /// [`adk_core::wrap_tool_result`] before it reaches the model, so returning
    /// a scalar is fine.
    async fn run(&self, args: Args, ctx: &ToolContext) -> Result<Value>;
}

/// A tool behind shared ownership, as agents and toolsets hold them.
pub type SharedTool = Arc<dyn Tool>;

/// Runs a tool with the framework behaviour ADK specifies around it:
/// confirmation gating, argument validation, and result normalization.
///
/// Agents call this rather than [`Tool::run`] directly.
pub async fn invoke_tool(tool: &dyn Tool, args: Args, ctx: &ToolContext) -> Result<Value> {
    if let Some(declaration) = tool.declaration() {
        declaration.validate_args(&Value::Object(args.clone()))?;
    }

    // Gate on confirmation before the body runs. An answer already in the
    // context means this is the resumed call, so the gate has been satisfied.
    if let Some(hint) = tool.confirmation_hint(&args) {
        match &ctx.tool_confirmation {
            None => return Err(ctx.request_confirmation(hint, None)),
            Some(confirmation) if !confirmation.confirmed => {
                return Err(AdkError::tool(
                    tool.name(),
                    "the user declined this tool call",
                ));
            }
            Some(_) => {}
        }
    }

    let result = tool.run(args, ctx).await?;
    Ok(adk_core::wrap_tool_result(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn conditional_policy_consults_the_arguments() {
        let policy = ConfirmationPolicy::Conditional(Box::new(|args: &Args| {
            let amount = args.get("amount").and_then(Value::as_i64).unwrap_or(0);
            (amount > 1000).then(|| format!("Approve a refund of {amount}?"))
        }));

        let mut small = Args::new();
        small.insert("amount".into(), json!(10));
        assert!(policy.hint_for(&small).is_none());

        let mut large = Args::new();
        large.insert("amount".into(), json!(5000));
        assert!(policy.hint_for(&large).unwrap().contains("5000"));
    }

    #[test]
    fn never_policy_never_asks() {
        assert!(ConfirmationPolicy::Never.hint_for(&Args::new()).is_none());
    }
}
