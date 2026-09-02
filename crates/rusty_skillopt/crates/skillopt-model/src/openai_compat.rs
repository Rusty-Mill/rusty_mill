use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use skillopt_core::{ChatBackend, Message, Role};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Works against any OpenAI-compatible `/chat/completions` endpoint:
/// OpenAI, Azure OpenAI (with the right `base_url`/deployment as `model`),
/// self-hosted servers, and local runners like Ollama (`base_url:
/// http://localhost:11434/v1`) that don't check auth at all.
pub struct OpenAiCompatBackend {
    client: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    model: String,
    temperature: Option<f32>,
    max_tokens: u32,
}

impl OpenAiCompatBackend {
    /// `api_key: None` sends no `Authorization` header at all, for servers
    /// (Ollama, most local OpenAI-compatible runners) that don't require one.
    pub fn new(
        api_key: Option<String>,
        base_url: Option<String>,
        model: String,
        temperature: Option<f32>,
        max_tokens: u32,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model,
            temperature,
            max_tokens,
        }
    }
}

#[derive(Serialize)]
struct OaMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct OaRequest {
    model: String,
    messages: Vec<OaMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct OaResponse {
    choices: Vec<OaChoice>,
}

#[derive(Deserialize)]
struct OaChoice {
    message: OaResponseMessage,
}

#[derive(Deserialize)]
struct OaResponseMessage {
    content: Option<String>,
}

#[async_trait]
impl ChatBackend for OpenAiCompatBackend {
    fn name(&self) -> &str {
        "openai_compatible"
    }

    async fn chat(&self, messages: &[Message]) -> anyhow::Result<String> {
        let oa_messages: Vec<OaMessage> = messages
            .iter()
            .map(|m| OaMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                content: m.content.clone(),
            })
            .collect();

        let req = OaRequest {
            model: self.model.clone(),
            messages: oa_messages,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };

        let mut req_builder = self
            .client
            .post(format!("{}/chat/completions", self.base_url));
        if let Some(api_key) = &self.api_key {
            req_builder = req_builder.bearer_auth(api_key);
        }
        let resp = req_builder.json(&req).send().await?;

        let status = resp.status();
        let body = resp.text().await?;
        anyhow::ensure!(
            status.is_success(),
            "openai-compatible API error ({status}): {body}"
        );

        let parsed: OaResponse = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!("failed to parse openai-compatible response: {e}\nbody: {body}")
        })?;

        let text = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| {
                anyhow::anyhow!("openai-compatible response contained no choices: {body}")
            })?;

        Ok(text)
    }
}
