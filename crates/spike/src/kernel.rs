//! The kernel: a hand-rolled agent loop over `&dyn ToolDispatch` (PRD 01).
//!
//! We do NOT reuse aisdk's `generate_text()` loop because it dispatches tools
//! through the `pub(crate)` `handle_tool_call`, leaving no policy interception
//! point. Instead the kernel calls a [`ChatModel`] one step at a time, vets +
//! dispatches any tool calls itself, feeds results back, and re-loops. aisdk is
//! adapted to [`ChatModel`] in [`crate::aisdk_adapter`] (the single place an
//! aisdk model type is named).

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ModelError;
use crate::tool::ToolDispatch;

/// One message in the conversation the kernel maintains.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    /// Human input.
    User(String),
    /// Model text output.
    Assistant(String),
    /// A tool result fed back to the model.
    ToolResult {
        /// Tool that produced this result.
        name: String,
        /// Rendered (model-facing) result string.
        content: String,
    },
}

/// One unit of model output in a single step: either text or a tool call.
#[derive(Debug, Clone)]
pub enum ModelStep {
    /// Final / interstitial text.
    Text(String),
    /// A request to invoke `name` with `args`.
    ToolCall {
        /// Tool name.
        name: String,
        /// JSON arguments.
        args: Value,
    },
}

/// The harness's model port. A single provider call: given the system prompt,
/// history, and advertised tool schemas, return the model's output steps. It
/// does NOT execute tools — the kernel does.
#[async_trait]
pub trait ChatModel: Send + Sync {
    /// Perform one model call.
    async fn respond(
        &self,
        system: &str,
        history: &[ChatMessage],
        tools: &[(String, Value)],
    ) -> Result<Vec<ModelStep>, ModelError>;
}

/// Drive one turn to completion: loop model→(vet+dispatch tools)→model until the
/// model stops calling tools. Returns the final assistant text.
pub async fn run_turn(
    model: &dyn ChatModel,
    dispatch: &dyn ToolDispatch,
    system: &str,
    history: &mut Vec<ChatMessage>,
) -> Result<String, ModelError> {
    let schemas = dispatch.schemas();
    let mut final_text = String::new();
    loop {
        let steps = model.respond(system, history, &schemas).await?;
        let mut called_tool = false;
        for step in steps {
            match step {
                ModelStep::Text(text) => {
                    final_text = text.clone();
                    history.push(ChatMessage::Assistant(text));
                }
                ModelStep::ToolCall { name, args } => {
                    called_tool = true;
                    let outcome = dispatch.dispatch(&name, args).await;
                    history.push(ChatMessage::ToolResult {
                        name,
                        content: outcome.render(),
                    });
                }
            }
        }
        if !called_tool {
            return Ok(final_text);
        }
    }
}
