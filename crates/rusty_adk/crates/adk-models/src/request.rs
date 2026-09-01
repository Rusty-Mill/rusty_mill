//! The request and response types exchanged with a model provider.

use adk_core::{Content, FunctionDeclaration, Schema};
use serde::{Deserialize, Serialize};

/// Sampling and safety knobs, mirroring `GenerateContentConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerateContentConfig {
    /// Sampling temperature. Lower is more deterministic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Nucleus sampling cutoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,

    /// Top-k sampling cutoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,

    /// Cap on generated tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,

    /// Sequences that end generation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,

    /// Token budget for models that expose explicit reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<u32>,

    /// Provider-specific settings passed through untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl GenerateContentConfig {
    /// Sets the sampling temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Sets the output token cap.
    pub fn with_max_output_tokens(mut self, max: u32) -> Self {
        self.max_output_tokens = Some(max);
        self
    }

    /// Sets the reasoning token budget.
    pub fn with_thinking_budget(mut self, budget: u32) -> Self {
        self.thinking_budget = Some(budget);
        self
    }
}

/// One request to a model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    /// Provider model identifier, e.g. `gemini-flash-latest`.
    pub model: String,

    /// Conversation history, oldest first.
    #[serde(default)]
    pub contents: Vec<Content>,

    /// System-level instruction, assembled from the agent's global and
    /// per-agent instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<String>,

    /// Tools the model may call this turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<FunctionDeclaration>,

    /// Sampling and safety configuration.
    #[serde(default)]
    pub config: GenerateContentConfig,

    /// Schema the response must conform to, for structured output.
    ///
    /// Most providers refuse to combine this with tools; the agent rejects
    /// that combination before it reaches the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Schema>,
}

impl LlmRequest {
    /// Builds a request for the named model.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Sets the conversation history.
    pub fn with_contents(mut self, contents: Vec<Content>) -> Self {
        self.contents = contents;
        self
    }

    /// Appends one content entry.
    pub fn push_content(mut self, content: Content) -> Self {
        self.contents.push(content);
        self
    }

    /// Sets the system instruction.
    pub fn with_system_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.system_instruction = Some(instruction.into());
        self
    }

    /// Sets the callable tools.
    pub fn with_tools(mut self, tools: Vec<FunctionDeclaration>) -> Self {
        self.tools = tools;
        self
    }

    /// Sets the sampling configuration.
    pub fn with_config(mut self, config: GenerateContentConfig) -> Self {
        self.config = config;
        self
    }

    /// Requires the response to match a schema.
    pub fn with_response_schema(mut self, schema: Schema) -> Self {
        self.response_schema = Some(schema);
        self
    }
}

/// Token accounting reported by the provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageMetadata {
    /// Tokens in the request.
    #[serde(default)]
    pub prompt_tokens: u32,
    /// Tokens generated.
    #[serde(default)]
    pub candidates_tokens: u32,
    /// Reasoning tokens, where the provider separates them.
    #[serde(default)]
    pub thoughts_tokens: u32,
    /// Total billed tokens.
    #[serde(default)]
    pub total_tokens: u32,
}

/// One response, or one streaming chunk, from a model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmResponse {
    /// The generated content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,

    /// `true` for an incomplete streaming chunk.
    #[serde(default)]
    pub partial: bool,

    /// `true` on the chunk that completes the turn.
    #[serde(default)]
    pub turn_complete: bool,

    /// Why generation stopped, e.g. `STOP`, `MAX_TOKENS`, `SAFETY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,

    /// Token accounting, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageMetadata>,

    /// Provider error code, when the call failed recoverably.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Provider error detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl LlmResponse {
    /// Builds a complete text response.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: Some(Content::model_text(text)),
            turn_complete: true,
            finish_reason: Some("STOP".into()),
            ..Default::default()
        }
    }

    /// Builds a complete response carrying arbitrary content.
    pub fn from_content(content: Content) -> Self {
        Self {
            content: Some(content),
            turn_complete: true,
            finish_reason: Some("STOP".into()),
            ..Default::default()
        }
    }

    /// Builds a partial streaming chunk.
    pub fn chunk(text: impl Into<String>) -> Self {
        Self {
            content: Some(Content::model_text(text)),
            partial: true,
            ..Default::default()
        }
    }

    /// Builds an error response.
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error_code: Some(code.into()),
            error_message: Some(message.into()),
            turn_complete: true,
            ..Default::default()
        }
    }

    /// The response's text, or an empty string.
    pub fn text_content(&self) -> String {
        self.content.as_ref().map(Content::text).unwrap_or_default()
    }

    /// Whether the model asked to call at least one tool.
    pub fn has_function_calls(&self) -> bool {
        self.content
            .as_ref()
            .map(|c| !c.function_calls().is_empty())
            .unwrap_or(false)
    }

    /// Whether this response reports an error.
    pub fn is_error(&self) -> bool {
        self.error_code.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{FunctionCall, Part, Role};

    #[test]
    fn text_response_is_complete_and_tool_free() {
        let r = LlmResponse::text("Paris");
        assert_eq!(r.text_content(), "Paris");
        assert!(r.turn_complete);
        assert!(!r.partial);
        assert!(!r.has_function_calls());
    }

    #[test]
    fn function_call_is_detected() {
        let r = LlmResponse::from_content(Content::new(
            Role::Model,
            vec![Part::FunctionCall(FunctionCall::new(
                "get_weather",
                Default::default(),
            ))],
        ));
        assert!(r.has_function_calls());
    }

    #[test]
    fn chunks_are_partial_and_incomplete() {
        let c = LlmResponse::chunk("Par");
        assert!(c.partial);
        assert!(!c.turn_complete);
    }
}
