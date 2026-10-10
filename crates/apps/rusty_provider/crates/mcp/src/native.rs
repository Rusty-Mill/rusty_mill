//! MCP tools that wrap rusty_provider's own routing capability directly --
//! Direction A ("rusty_provider as an MCP server") from the design doc.
//!
//! Each tool's argument struct is deliberately smaller than the full
//! `rp_core::ChatRequest`/`EmbeddingsRequest` wire shape: those types already
//! derive `Deserialize` with `#[serde(default)]` on every optional field, so
//! building a `serde_json::Value` from the tool args and deserializing it
//! into the real request type reuses that logic exactly instead of
//! duplicating two dozen fields here.
//!
//! The server's handlers are blocking, so each one runs the router's async
//! call on the tokio runtime it was registered with ([`register`]); call
//! them from a thread that is not itself a runtime worker (the HTTP and
//! stdio transports use threads of their own).

use std::sync::Arc;

use rp_router::Router;
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{
    CallToolParams, CallToolResult, ContentBlock, ErrorCode, ErrorData, Tool,
};
use rusty_mcp_server::ServerBuilder;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

/// One chat message, the minimal shape a tool caller needs to supply --
/// converted into a full `rp_core::ChatMessage` via JSON deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpChatMessage {
    /// `"system"`, `"user"`, `"assistant"`, or `"tool"`.
    pub role: String,
    /// Message text.
    pub content: String,
}

/// Arguments for the `chat_completion` tool.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Serialize)]
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
#[derive(Debug, Deserialize)]
pub struct EmbeddingsArgs {
    /// "provider/model" or a `[[routes]]` alias.
    pub model: String,
    /// Text(s) to embed.
    pub input: Vec<String>,
}

/// One embedding vector, positioned to match `EmbeddingsArgs.input`.
#[derive(Debug, Serialize)]
pub struct EmbeddingOutput {
    pub index: usize,
    pub embedding: Vec<f32>,
}

/// One model this router can currently reach.
#[derive(Debug, Serialize)]
pub struct ModelSummary {
    pub id: String,
    pub owned_by: String,
}

const CHAT_INPUT: &str = r#"{"type":"object","properties":{
  "model":{"type":"string","description":"Either \"provider/model\" (e.g. \"anthropic/claude-sonnet-5\") to target one provider directly, or a router alias configured in [[routes]]."},
  "messages":{"type":"array","description":"Conversation so far, oldest first.","items":{"type":"object","properties":{
    "role":{"type":"string","description":"\"system\", \"user\", \"assistant\", or \"tool\"."},
    "content":{"type":"string","description":"Message text."}},"required":["role","content"]}},
  "temperature":{"type":["number","null"],"description":"Sampling temperature. Provider default if unset."},
  "max_tokens":{"type":["integer","null"],"minimum":0,"description":"Maximum tokens to generate. Provider default if unset."}},
 "required":["model","messages"]}"#;

const CHAT_OUTPUT: &str = r#"{"type":"object","properties":{
  "content":{"type":"string"},"model":{"type":"string"},
  "prompt_tokens":{"type":"integer","minimum":0},"completion_tokens":{"type":"integer","minimum":0},
  "cost_usd":{"type":"number"}},
 "required":["content","model","prompt_tokens","completion_tokens"]}"#;

const EMBEDDINGS_INPUT: &str = r#"{"type":"object","properties":{
  "model":{"type":"string","description":"\"provider/model\" or a [[routes]] alias."},
  "input":{"type":"array","items":{"type":"string"},"description":"Text(s) to embed."}},
 "required":["model","input"]}"#;

const EMBEDDINGS_OUTPUT: &str = r#"{"type":"array","items":{"type":"object","properties":{
  "index":{"type":"integer","minimum":0},"embedding":{"type":"array","items":{"type":"number"}}},
 "required":["index","embedding"]}}"#;

const LIST_MODELS_INPUT: &str = r#"{"type":"object","properties":{}}"#;

const LIST_MODELS_OUTPUT: &str = r#"{"type":"array","items":{"type":"object","properties":{
  "id":{"type":"string"},"owned_by":{"type":"string"}},"required":["id","owned_by"]}}"#;

fn definition(
    name: &str,
    description: &str,
    input: &str,
    output: &str,
) -> Result<Tool, rusty_mcp_server::json::Error> {
    let mut tool = Tool::new(name, Value::from_json_str(input)?);
    tool.description = Some(description.to_owned());
    tool.output_schema = Some(Value::from_json_str(output)?);
    Ok(tool)
}

fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, message)
}

/// The call's arguments as `T` (an absent `arguments` is an empty object).
fn arguments<T: serde::de::DeserializeOwned>(call: &CallToolParams) -> Result<T, ErrorData> {
    let text = call
        .arguments
        .as_ref()
        .map_or_else(|| "{}".to_owned(), Value::to_json_string);
    serde_json::from_str(&text).map_err(|e| invalid(format!("invalid arguments: {e}")))
}

/// A result carrying `output` as structured content and as its JSON text.
fn structured<T: Serialize>(output: &T) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string(output)
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string()))?;
    let value = Value::from_json_str(&text)
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string()))?;
    Ok(CallToolResult {
        content: vec![ContentBlock::text(text)],
        structured_content: Some(value),
        ..CallToolResult::default()
    })
}

/// Offer `chat_completion`, `list_models` and `embeddings` on `builder`, run
/// against `router` on the runtime `rt`.
///
/// # Errors
/// If a built-in schema does not parse (a bug, caught by this crate's tests).
pub fn register(
    builder: ServerBuilder,
    router: &Arc<Router>,
    rt: &Handle,
) -> Result<ServerBuilder, rusty_mcp_server::json::Error> {
    let chat = definition(
        "chat_completion",
        "Send a chat completion request through rusty_provider's own routing (fallback chains, caching, free-tier tracking all apply).",
        CHAT_INPUT,
        CHAT_OUTPUT,
    )?;
    let models = definition(
        "list_models",
        "List the models rusty_provider currently has pricing/routing info for.",
        LIST_MODELS_INPUT,
        LIST_MODELS_OUTPUT,
    )?;
    let embeddings = definition(
        "embeddings",
        "Embed one or more texts through rusty_provider's own routing.",
        EMBEDDINGS_INPUT,
        EMBEDDINGS_OUTPUT,
    )?;
    let (r1, h1) = (Arc::clone(router), rt.clone());
    let (r2, h2) = (Arc::clone(router), rt.clone());
    let (r3, h3) = (Arc::clone(router), rt.clone());
    Ok(builder
        .tool(chat, move |_ctx, call| {
            chat_completion(&r1, &h1, &arguments(&call)?)
        })
        .tool(models, move |_ctx, _call| list_models(&r2, &h2))
        .tool(embeddings, move |_ctx, call| {
            embeddings_call(&r3, &h3, arguments(&call)?)
        }))
}

fn chat_completion(
    router: &Router,
    rt: &Handle,
    args: &ChatCompletionArgs,
) -> Result<CallToolResult, ErrorData> {
    let value = serde_json::json!({
        "model": args.model,
        "messages": args.messages,
        "temperature": args.temperature,
        "max_tokens": args.max_tokens,
    });
    let request: rp_core::ChatRequest =
        serde_json::from_value(value).map_err(|e| invalid(format!("invalid chat request: {e}")))?;
    let response = rt
        .block_on(router.dispatch(&request))
        .map_err(|e| invalid(e.to_string()))?;

    let content = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .map(|c| c.as_plain_text())
        .unwrap_or_default();
    let usage = response.usage.unwrap_or_default();
    structured(&ChatCompletionOutput {
        content,
        model: response.model,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        cost_usd: response.cost_usd,
    })
}

fn list_models(router: &Router, _rt: &Handle) -> Result<CallToolResult, ErrorData> {
    let models: Vec<ModelSummary> = router
        .priced_models()
        .into_iter()
        .map(|m| ModelSummary {
            id: m.id,
            owned_by: m.owned_by,
        })
        .collect();
    structured(&models)
}

fn embeddings_call(
    router: &Router,
    rt: &Handle,
    args: EmbeddingsArgs,
) -> Result<CallToolResult, ErrorData> {
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
    let response = rt
        .block_on(router.embeddings(&request))
        .map_err(|e| invalid(e.to_string()))?;
    let data: Vec<EmbeddingOutput> = response
        .data
        .into_iter()
        .map(|d| EmbeddingOutput {
            index: d.index,
            embedding: d.embedding,
        })
        .collect();
    structured(&data)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_schema_parses() {
        for (input, output) in [
            (CHAT_INPUT, CHAT_OUTPUT),
            (EMBEDDINGS_INPUT, EMBEDDINGS_OUTPUT),
            (LIST_MODELS_INPUT, LIST_MODELS_OUTPUT),
        ] {
            assert!(definition("t", "d", input, output).is_ok());
        }
    }

    #[test]
    fn absent_arguments_are_an_empty_object() {
        let call = CallToolParams::new("list_models");
        let args: serde_json::Map<String, serde_json::Value> = arguments(&call).unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn malformed_arguments_are_invalid_params() {
        let mut call = CallToolParams::new("embeddings");
        call.arguments = Some(Value::from_json_str(r#"{"model":1}"#).unwrap());
        let err = arguments::<EmbeddingsArgs>(&call).unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }
}
