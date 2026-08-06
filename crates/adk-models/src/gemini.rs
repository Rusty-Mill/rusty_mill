//! Google Gemini connector, over the Generative Language API.
//!
//! ADK's own content model follows `google.genai`, so this connector is mostly
//! a direct projection: [`Content`] and [`Part`] map one-to-one onto the wire
//! shape, and [`adk_core::Schema`] already serializes with the upper-case type
//! names the API expects.

use adk_core::{AdkError, Content, FunctionCall, Part, Result, Role};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::model::Model;
use crate::request::{LlmRequest, LlmResponse, UsageMetadata};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// A [`Model`] backed by the Gemini Generative Language API.
pub struct GeminiModel {
    model: String,
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl GeminiModel {
    /// Builds a connector for `model`, reading the key from `GOOGLE_API_KEY`
    /// or `GEMINI_API_KEY`.
    pub fn from_env(model: impl Into<String>) -> Result<Self> {
        let api_key = std::env::var("GOOGLE_API_KEY")
            .or_else(|_| std::env::var("GEMINI_API_KEY"))
            .map_err(|_| {
                AdkError::Config("neither GOOGLE_API_KEY nor GEMINI_API_KEY is set".into())
            })?;
        Ok(Self::new(model, api_key))
    }

    /// Builds a connector with an explicit key.
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Points the connector at a different base URL, for proxies and tests.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn endpoint(&self, model: &str) -> String {
        format!("{}/models/{}:generateContent", self.base_url, model)
    }

    /// Translates an ADK request into a `generateContent` body.
    pub fn build_body(&self, request: &LlmRequest) -> Value {
        let mut body = Map::new();

        body.insert(
            "contents".into(),
            Value::Array(request.contents.iter().map(to_gemini_content).collect()),
        );

        if let Some(system) = &request.system_instruction {
            body.insert(
                "systemInstruction".into(),
                json!({"parts": [{"text": system}]}),
            );
        }

        if !request.tools.is_empty() {
            let declarations: Vec<Value> = request
                .tools
                .iter()
                .map(|decl| {
                    let mut entry = Map::new();
                    entry.insert("name".into(), json!(decl.name));
                    entry.insert("description".into(), json!(decl.description));
                    if let Some(params) = &decl.parameters {
                        entry.insert(
                            "parameters".into(),
                            serde_json::to_value(params).unwrap_or_else(|_| json!({})),
                        );
                    }
                    Value::Object(entry)
                })
                .collect();
            body.insert(
                "tools".into(),
                json!([{"functionDeclarations": declarations}]),
            );
        }

        let mut generation = Map::new();
        let config = &request.config;
        if let Some(t) = config.temperature {
            generation.insert("temperature".into(), json!(t));
        }
        if let Some(p) = config.top_p {
            generation.insert("topP".into(), json!(p));
        }
        if let Some(k) = config.top_k {
            generation.insert("topK".into(), json!(k));
        }
        if let Some(max) = config.max_output_tokens {
            generation.insert("maxOutputTokens".into(), json!(max));
        }
        if !config.stop_sequences.is_empty() {
            generation.insert("stopSequences".into(), json!(config.stop_sequences));
        }
        if let Some(budget) = config.thinking_budget {
            generation.insert("thinkingConfig".into(), json!({"thinkingBudget": budget}));
        }
        if let Some(schema) = &request.response_schema {
            generation.insert("responseMimeType".into(), json!("application/json"));
            generation.insert(
                "responseSchema".into(),
                serde_json::to_value(schema).unwrap_or_else(|_| json!({})),
            );
        }
        if !generation.is_empty() {
            body.insert("generationConfig".into(), Value::Object(generation));
        }

        if let Some(Value::Object(extra)) = &config.extra {
            for (key, value) in extra {
                body.insert(key.clone(), value.clone());
            }
        }

        Value::Object(body)
    }

    /// Translates a `generateContent` response body into an [`LlmResponse`].
    pub fn parse_response(&self, body: &Value) -> Result<LlmResponse> {
        if let Some(message) = body.pointer("/error/message").and_then(Value::as_str) {
            return Err(AdkError::model(&self.model, message));
        }

        let candidate = body
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|c| c.first());

        let finish_reason = candidate
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str)
            .map(str::to_string);

        // A prompt blocked by safety filters returns no candidate at all; the
        // reason lives on promptFeedback instead.
        if candidate.is_none() {
            let blocked = body
                .pointer("/promptFeedback/blockReason")
                .and_then(Value::as_str);
            return Ok(LlmResponse {
                turn_complete: true,
                error_code: Some("NO_CANDIDATES".into()),
                error_message: Some(match blocked {
                    Some(reason) => format!("prompt blocked: {reason}"),
                    None => "model returned no candidates".to_string(),
                }),
                ..Default::default()
            });
        }

        let mut parts = Vec::new();
        for part in candidate
            .and_then(|c| c.pointer("/content/parts"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                // A part flagged as thought carries reasoning, not answer text.
                if part.get("thought").and_then(Value::as_bool).unwrap_or(false) {
                    parts.push(Part::Thought(text.to_string()));
                } else {
                    parts.push(Part::Text(text.to_string()));
                }
            } else if let Some(call) = part.get("functionCall") {
                parts.push(Part::FunctionCall(FunctionCall {
                    id: call.get("id").and_then(Value::as_str).map(str::to_string),
                    name: call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    args: call
                        .get("args")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                }));
            }
        }

        let usage = body.get("usageMetadata").map(|u| {
            let field = |name: &str| u.get(name).and_then(Value::as_u64).unwrap_or(0) as u32;
            UsageMetadata {
                prompt_tokens: field("promptTokenCount"),
                candidates_tokens: field("candidatesTokenCount"),
                thoughts_tokens: field("thoughtsTokenCount"),
                total_tokens: field("totalTokenCount"),
            }
        });

        Ok(LlmResponse {
            content: (!parts.is_empty()).then(|| Content::new(Role::Model, parts)),
            partial: false,
            turn_complete: true,
            finish_reason,
            usage,
            error_code: None,
            error_message: None,
        })
    }
}

fn to_gemini_content(content: &Content) -> Value {
    let parts: Vec<Value> = content
        .parts
        .iter()
        // Every ADK part has a Gemini equivalent, so this is a total mapping.
        .map(|part| match part {
            Part::Text(text) => json!({"text": text}),
            Part::Thought(text) => json!({"text": text, "thought": true}),
            Part::InlineData(blob) => json!({
                "inlineData": {"mimeType": blob.mime_type, "data": blob.data},
            }),
            Part::FileData(file) => json!({
                "fileData": {"mimeType": file.mime_type, "fileUri": file.file_uri},
            }),
            Part::FunctionCall(call) => json!({
                "functionCall": {"name": call.name, "args": Value::Object(call.args.clone())},
            }),
            Part::FunctionResponse(response) => json!({
                "functionResponse": {"name": response.name, "response": response.response},
            }),
        })
        .collect();

    json!({"role": content.role.as_str(), "parts": parts})
}

#[async_trait]
impl Model for GeminiModel {
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
            .post(self.endpoint(&request.model))
            .header("x-goog-api-key", &self.api_key)
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

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{FunctionDeclaration, Schema};

    fn model() -> GeminiModel {
        GeminiModel::new("gemini-flash-latest", "test-key")
    }

    #[test]
    fn system_instruction_uses_the_parts_wrapper() {
        let request = LlmRequest::new("m").with_system_instruction("be terse");
        let body = model().build_body(&request);
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be terse");
    }

    #[test]
    fn tools_are_nested_under_function_declarations() {
        let request = LlmRequest::new("m").with_tools(vec![FunctionDeclaration::new(
            "get_weather",
            "Gets weather.",
        )
        .with_parameters(Schema::object().property("city", Schema::string()))]);
        let body = model().build_body(&request);
        let decl = &body["tools"][0]["functionDeclarations"][0];
        assert_eq!(decl["name"], "get_weather");
        // Gemini expects the upper-case type names ADK's Schema already emits.
        assert_eq!(decl["parameters"]["type"], "OBJECT");
    }

    #[test]
    fn empty_generation_config_is_omitted() {
        let body = model().build_body(&LlmRequest::new("m"));
        assert!(body.get("generationConfig").is_none());
    }

    #[test]
    fn parses_text_and_function_calls() {
        let payload = json!({
            "candidates": [{
                "content": {"parts": [
                    {"text": "Checking."},
                    {"functionCall": {"name": "get_weather", "args": {"city": "Tokyo"}}},
                ]},
                "finishReason": "STOP",
            }],
            "usageMetadata": {"promptTokenCount": 8, "candidatesTokenCount": 4, "totalTokenCount": 12},
        });
        let parsed = model().parse_response(&payload).unwrap();
        assert_eq!(parsed.text_content(), "Checking.");
        assert!(parsed.has_function_calls());
        assert_eq!(parsed.usage.unwrap().total_tokens, 12);
    }

    #[test]
    fn thought_parts_are_kept_out_of_the_answer_text() {
        let payload = json!({
            "candidates": [{"content": {"parts": [
                {"text": "reasoning", "thought": true},
                {"text": "answer"},
            ]}}],
        });
        let parsed = model().parse_response(&payload).unwrap();
        assert_eq!(parsed.text_content(), "answer");
        assert_eq!(parsed.content.unwrap().parts.len(), 2);
    }

    #[test]
    fn a_blocked_prompt_reports_its_reason() {
        let payload = json!({"promptFeedback": {"blockReason": "SAFETY"}});
        let parsed = model().parse_response(&payload).unwrap();
        assert!(parsed.is_error());
        assert!(parsed.error_message.unwrap().contains("SAFETY"));
    }

    #[test]
    fn an_error_envelope_becomes_a_model_error() {
        let payload = json!({"error": {"message": "quota exceeded"}});
        assert!(model().parse_response(&payload).is_err());
    }

    #[test]
    fn endpoint_targets_the_generate_content_method() {
        assert!(model()
            .endpoint("gemini-flash-latest")
            .ends_with("/models/gemini-flash-latest:generateContent"));
    }
}
