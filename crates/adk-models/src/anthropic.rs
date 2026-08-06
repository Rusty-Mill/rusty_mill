//! Anthropic Claude connector, over the Messages API.
//!
//! There is no official Anthropic SDK for Rust, so this speaks the documented
//! HTTP surface directly: `POST https://api.anthropic.com/v1/messages` with an
//! `x-api-key` header and `anthropic-version: 2023-06-01`.
//!
//! # Sampling parameters
//!
//! Current Claude models reject `temperature`, `top_p`, and `top_k` with a 400.
//! [`GenerateContentConfig`] holds them as `Option`, and this connector sends a
//! field only when the caller set it — so the default configuration is valid on
//! every model, and a caller who sets them has explicitly opted in.

use adk_core::{AdkError, Args, Content, FunctionCall, FunctionResponse, Part, Result, Role};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::model::Model;
use crate::request::{LlmRequest, LlmResponse, UsageMetadata};

const DEFAULT_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

/// The `max_tokens` sent when the caller does not set one.
///
/// The Messages API requires the field, so a default is unavoidable; this value
/// leaves room for a substantial answer without risking an HTTP timeout on a
/// non-streaming request.
const DEFAULT_MAX_TOKENS: u32 = 16_000;

/// A [`Model`] backed by the Anthropic Messages API.
pub struct AnthropicModel {
    model: String,
    api_key: String,
    endpoint: String,
    client: reqwest::Client,
}

impl AnthropicModel {
    /// Builds a connector for `model`, reading the key from `ANTHROPIC_API_KEY`.
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            AdkError::Config("ANTHROPIC_API_KEY is not set".into())
        })?;
        Ok(Self::new(model, api_key))
    }

    /// Builds a connector with an explicit key.
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            endpoint: DEFAULT_ENDPOINT.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Points the connector at a different base URL, for proxies and tests.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Translates an ADK request into a Messages API body.
    pub fn build_body(&self, request: &LlmRequest) -> Value {
        let mut body = Map::new();
        body.insert("model".into(), json!(request.model));
        body.insert(
            "max_tokens".into(),
            json!(request.config.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
        );

        if let Some(system) = &request.system_instruction {
            body.insert("system".into(), json!(system));
        }

        body.insert(
            "messages".into(),
            Value::Array(request.contents.iter().map(to_anthropic_message).collect()),
        );

        if !request.tools.is_empty() {
            let tools: Vec<Value> = request
                .tools
                .iter()
                .map(|decl| {
                    json!({
                        "name": decl.name,
                        "description": decl.description,
                        "input_schema": to_input_schema(decl.parameters.as_ref()),
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(tools));
        }

        // Only send sampling parameters the caller explicitly set: current
        // models reject them outright, so a synthesized default would break
        // every request.
        let config = &request.config;
        if let Some(t) = config.temperature {
            body.insert("temperature".into(), json!(t));
        }
        if let Some(p) = config.top_p {
            body.insert("top_p".into(), json!(p));
        }
        if let Some(k) = config.top_k {
            body.insert("top_k".into(), json!(k));
        }
        if !config.stop_sequences.is_empty() {
            body.insert("stop_sequences".into(), json!(config.stop_sequences));
        }
        if let Some(Value::Object(extra)) = &config.extra {
            for (key, value) in extra {
                body.insert(key.clone(), value.clone());
            }
        }

        Value::Object(body)
    }

    /// Translates a Messages API response body into an [`LlmResponse`].
    pub fn parse_response(&self, body: &Value) -> Result<LlmResponse> {
        if body.get("type").and_then(Value::as_str) == Some("error") {
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(AdkError::model(&self.model, message));
        }

        let stop_reason = body
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string);

        // A safety decline arrives as a normal 200 with `stop_reason: refusal`
        // and no usable content, so it must be checked before reading blocks.
        if stop_reason.as_deref() == Some("refusal") {
            let category = body
                .pointer("/stop_details/category")
                .and_then(Value::as_str)
                .unwrap_or("unspecified");
            return Ok(LlmResponse {
                finish_reason: stop_reason,
                error_code: Some("REFUSAL".into()),
                error_message: Some(format!("model declined the request ({category})")),
                turn_complete: true,
                ..Default::default()
            });
        }

        let mut parts = Vec::new();
        for block in body
            .get("content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        parts.push(Part::Text(text.to_string()));
                    }
                }
                Some("thinking") => {
                    if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                        // Thinking text is empty unless the caller opted into
                        // `display: summarized`; skip empty blocks rather than
                        // emitting a contentless part.
                        if !text.is_empty() {
                            parts.push(Part::Thought(text.to_string()));
                        }
                    }
                }
                Some("tool_use") => {
                    let args = block
                        .get("input")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    parts.push(Part::FunctionCall(FunctionCall {
                        id: block.get("id").and_then(Value::as_str).map(str::to_string),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        args,
                    }));
                }
                _ => {}
            }
        }

        let usage = body.get("usage").map(|u| UsageMetadata {
            prompt_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            candidates_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            thoughts_tokens: 0,
            total_tokens: (u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0)
                + u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0))
                as u32,
        });

        Ok(LlmResponse {
            content: (!parts.is_empty()).then(|| Content::new(Role::Model, parts)),
            partial: false,
            turn_complete: true,
            finish_reason: stop_reason,
            usage,
            error_code: None,
            error_message: None,
        })
    }
}

/// Maps ADK content onto a Messages API message.
///
/// Tool results travel as `tool_result` blocks in a `user` message, which is
/// how the Messages API expects them — ADK's own convention of putting function
/// responses on the user role lines up exactly.
fn to_anthropic_message(content: &Content) -> Value {
    let blocks: Vec<Value> = content
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(json!({"type": "text", "text": text})),
            Part::FunctionCall(call) => Some(json!({
                "type": "tool_use",
                "id": call.id.clone().unwrap_or_else(|| adk_core::new_id("toolu")),
                "name": call.name,
                "input": Value::Object(call.args.clone()),
            })),
            Part::FunctionResponse(response) => Some(json!({
                "type": "tool_result",
                "tool_use_id": response.id.clone().unwrap_or_default(),
                "content": response.response.to_string(),
            })),
            // Thinking blocks must be replayed verbatim or not at all; this
            // connector does not retain the signatures needed to replay them,
            // so it drops them rather than sending an edited block the API
            // would reject.
            Part::Thought(_) => None,
            Part::InlineData(_) | Part::FileData(_) => None,
        })
        .collect();

    json!({
        "role": match content.role {
            Role::User => "user",
            Role::Model => "assistant",
        },
        "content": blocks,
    })
}

/// Renders a parameter schema as the JSON Schema the Messages API expects.
///
/// Lower-cases the type names, since ADK's `Schema` serializes them upper-case
/// for the `google.genai` surface.
fn to_input_schema(schema: Option<&adk_core::Schema>) -> Value {
    let Some(schema) = schema else {
        return json!({"type": "object", "properties": {}});
    };
    lowercase_types(&serde_json::to_value(schema).unwrap_or_else(|_| json!({})))
}

fn lowercase_types(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, val)| {
                    if key == "type" {
                        if let Some(name) = val.as_str() {
                            return (key.clone(), json!(name.to_lowercase()));
                        }
                    }
                    (key.clone(), lowercase_types(val))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(lowercase_types).collect()),
        other => other.clone(),
    }
}

#[async_trait]
impl Model for AnthropicModel {
    fn name(&self) -> &str {
        &self.model
    }

    async fn generate_content(&self, mut request: LlmRequest) -> Result<LlmResponse> {
        if request.model.is_empty() {
            request.model = self.model.clone();
        }
        let body = self.build_body(&request);

        let response = self
            .client
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AdkError::model(&self.model, e.to_string()))?;

        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|e| AdkError::model(&self.model, format!("malformed response: {e}")))?;

        if !status.is_success() {
            let message = payload
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            return Err(AdkError::model(
                &self.model,
                format!("HTTP {status}: {message}"),
            ));
        }

        self.parse_response(&payload)
    }
}

/// Convenience alias for the argument map used when scripting tool calls.
pub type ToolArgs = Args;

/// Re-exported so callers can build responses without importing `adk-core`.
pub type ToolResponse = FunctionResponse;

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::Schema;
    use crate::request::GenerateContentConfig;

    fn model() -> AnthropicModel {
        AnthropicModel::new("claude-opus-5", "test-key")
    }

    #[test]
    fn sampling_params_are_omitted_unless_set() {
        // Current models reject these outright, so an unset config must not
        // synthesize them.
        let body = model().build_body(&LlmRequest::new("claude-opus-5"));
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert!(body.get("top_k").is_none());
    }

    #[test]
    fn sampling_params_are_sent_when_explicitly_set() {
        let request = LlmRequest::new("claude-opus-5")
            .with_config(GenerateContentConfig::default().with_temperature(0.2));
        let body = model().build_body(&request);
        // Compare with a tolerance: the config stores f32 and serde widens it.
        let sent = body["temperature"].as_f64().unwrap();
        assert!((sent - 0.2).abs() < 1e-6, "got {sent}");
    }

    #[test]
    fn max_tokens_is_always_present() {
        let body = model().build_body(&LlmRequest::new("claude-opus-5"));
        assert_eq!(body["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn tool_declarations_use_lowercase_schema_types() {
        let request = LlmRequest::new("claude-opus-5").with_tools(vec![
            adk_core::FunctionDeclaration::new("get_weather", "Gets weather.")
                .with_parameters(Schema::object().property("city", Schema::string())),
        ]);
        let body = model().build_body(&request);
        let schema = &body["tools"][0]["input_schema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["city"]["type"], "string");
        assert_eq!(body["tools"][0]["name"], "get_weather");
    }

    #[test]
    fn function_responses_become_tool_result_blocks_on_the_user_role() {
        let request = LlmRequest::new("m").push_content(Content::new(
            Role::User,
            vec![Part::FunctionResponse(FunctionResponse {
                id: Some("toolu_1".into()),
                name: "get_weather".into(),
                response: json!({"status": "success"}),
            })],
        ));
        let body = model().build_body(&request);
        let block = &body["messages"][0]["content"][0];
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["tool_use_id"], "toolu_1");
    }

    #[test]
    fn parses_text_and_tool_use_blocks() {
        let payload = json!({
            "content": [
                {"type": "text", "text": "Let me check."},
                {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "Paris"}},
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 5},
        });
        let parsed = model().parse_response(&payload).unwrap();
        assert_eq!(parsed.text_content(), "Let me check.");
        assert!(parsed.has_function_calls());
        let calls = parsed.content.as_ref().unwrap().function_calls();
        assert_eq!(calls[0].args["city"], "Paris");
        assert_eq!(parsed.usage.unwrap().total_tokens, 15);
    }

    #[test]
    fn a_refusal_is_reported_as_an_error_response_not_empty_content() {
        let payload = json!({
            "content": [],
            "stop_reason": "refusal",
            "stop_details": {"category": "cyber"},
        });
        let parsed = model().parse_response(&payload).unwrap();
        assert!(parsed.is_error());
        assert!(parsed.error_message.unwrap().contains("cyber"));
    }

    #[test]
    fn empty_thinking_blocks_are_dropped() {
        let payload = json!({
            "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
        });
        let parsed = model().parse_response(&payload).unwrap();
        assert_eq!(parsed.content.unwrap().parts.len(), 1);
    }

    #[test]
    fn an_error_envelope_becomes_a_model_error() {
        let payload = json!({"type": "error", "error": {"message": "overloaded"}});
        let err = model().parse_response(&payload).unwrap_err();
        assert!(err.to_string().contains("overloaded"));
    }
}
