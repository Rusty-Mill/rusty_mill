//! MCP tools that wrap rusty_provider's own routing capability directly --
//! Direction A ("rusty_provider as an MCP server") from the design doc.
//!
//! Each tool's argument struct is deliberately smaller than the full
//! `rp_core::ChatRequest`/`EmbeddingsRequest` wire shape: those types already
//! derive `Deserialize` with `#[serde(default)]` on every optional field, so
//! building a `serde_json::Value` from the tool args and deserializing it
//! into the real request type reuses that logic exactly instead of
//! duplicating two dozen fields here.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ErrorData;
use rmcp::{tool, tool_router, Json};
use rusty_mcp::ToolError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use rp_router::Router;

/// One chat message, the minimal shape a tool caller needs to supply --
/// converted into a full `rp_core::ChatMessage` via JSON deserialization.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpChatMessage {
    /// `"system"`, `"user"`, `"assistant"`, or `"tool"`.
    pub role: String,
    /// Message text.
    pub content: String,
}

/// Arguments for the `chat_completion` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChatCompletionArgs {
    /// Either `"provider/model"` (e.g. `"anthropic/claude-sonnet-5"`) to
    /// target one provider directly, or a router alias configured in
    /// `[[routes]]`.
    pub model: String,
    /// Conversation so far, oldest first.
    pub messages: Vec<McpChatMessage>,
    /// Sampling temperature. Provider default if unset.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Maximum tokens to generate. Provider default if unset.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// A chat completion result, trimmed to what a tool caller needs: the
/// reply text, which "provider/model" actually served it, and token usage.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ChatCompletionOutput {
    /// The assistant's reply text (empty if the model returned a tool call
    /// instead of text -- this tool doesn't expose `tools`/`tool_choice`).
    pub content: String,
    /// The fully-qualified "provider/model" that served the request.
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Arguments for the `embeddings` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmbeddingsArgs {
    /// "provider/model" or a `[[routes]]` alias.
    pub model: String,
    /// Text(s) to embed.
    pub input: Vec<String>,
}

/// One embedding vector, positioned to match `EmbeddingsArgs.input`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct EmbeddingOutput {
    pub index: usize,
    pub embedding: Vec<f32>,
}

/// Arguments for the `list_models` tool (currently none).
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListModelsArgs {}

/// One model this router can currently reach.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelSummary {
    pub id: String,
    pub owned_by: String,
}

/// Tools wrapping `rp_router::Router`'s own dispatch/embeddings/listing API.
#[derive(Clone)]
pub struct NativeTools {
    router: Arc<Router>,
    pub(crate) tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl NativeTools {
    pub fn new(router: Arc<Router>) -> Self {
        Self {
            router,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Send a chat completion request through rusty_provider's own routing (fallback chains, caching, free-tier tracking all apply)."
    )]
    pub async fn chat_completion(
        &self,
        Parameters(args): Parameters<ChatCompletionArgs>,
    ) -> Result<Json<ChatCompletionOutput>, ErrorData> {
        let value = serde_json::json!({
            "model": args.model,
            "messages": args.messages,
            "temperature": args.temperature,
            "max_tokens": args.max_tokens,
        });
        let request: rp_core::ChatRequest = serde_json::from_value(value)
            .map_err(|e| ToolError::invalid(format!("invalid chat request: {e}")))?;

        let response = self
            .router
            .dispatch(&request)
            .await
            .map_err(|e| ToolError::invalid(e.to_string()))?;

        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .map(|c| c.as_plain_text())
            .unwrap_or_default();
        let usage = response.usage.unwrap_or_default();

        Ok(Json(ChatCompletionOutput {
            content,
            model: response.model,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cost_usd: response.cost_usd,
        }))
    }

    #[tool(description = "List the models rusty_provider currently has pricing/routing info for.")]
    pub async fn list_models(
        &self,
        Parameters(_args): Parameters<ListModelsArgs>,
    ) -> Result<Json<Vec<ModelSummary>>, ErrorData> {
        let models = self
            .router
            .priced_models()
            .into_iter()
            .map(|m| ModelSummary {
                id: m.id,
                owned_by: m.owned_by,
            })
            .collect();
        Ok(Json(models))
    }

    #[tool(description = "Embed one or more texts through rusty_provider's own routing.")]
    pub async fn embeddings(
        &self,
        Parameters(args): Parameters<EmbeddingsArgs>,
    ) -> Result<Json<Vec<EmbeddingOutput>>, ErrorData> {
        let input = if args.input.len() == 1 {
            rp_core::EmbeddingsInput::Single(args.input[0].clone())
        } else {
            rp_core::EmbeddingsInput::Multiple(args.input)
        };
        let request = rp_core::EmbeddingsRequest {
            model: args.model,
            input,
            encoding_format: None,
            dimensions: None,
        };

        let response = self
            .router
            .embeddings(&request)
            .await
            .map_err(|e| ToolError::invalid(e.to_string()))?;

        Ok(Json(
            response
                .data
                .into_iter()
                .map(|d| EmbeddingOutput {
                    index: d.index,
                    embedding: d.embedding,
                })
                .collect(),
        ))
    }
}
