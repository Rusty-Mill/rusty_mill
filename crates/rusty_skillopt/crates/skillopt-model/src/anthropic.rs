use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use skillopt_core::{ChatBackend, Message, Role};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicBackend {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    temperature: Option<f32>,
    max_tokens: u32,
}

impl AnthropicBackend {
    pub fn new(
        api_key: String,
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
struct AnthropicMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[async_trait]
impl ChatBackend for AnthropicBackend {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, messages: &[Message]) -> anyhow::Result<String> {
        let mut system_parts = Vec::new();
        let mut turns = Vec::new();
        for m in messages {
            match m.role {
                Role::System => system_parts.push(m.content.clone()),
                Role::User => turns.push(AnthropicMessage {
                    role: "user",
                    content: m.content.clone(),
                }),
                Role::Assistant => turns.push(AnthropicMessage {
                    role: "assistant",
                    content: m.content.clone(),
                }),
            }
        }
        anyhow::ensure!(
            !turns.is_empty(),
            "anthropic chat requires at least one user/assistant message"
        );

        let req = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            messages: turns,
            system: (!system_parts.is_empty()).then(|| system_parts.join("\n\n")),
            temperature: self.temperature,
        };

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        anyhow::ensure!(
            status.is_success(),
            "anthropic API error ({status}): {body}"
        );

        let parsed: AnthropicResponse = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!("failed to parse anthropic response: {e}\nbody: {body}")
        })?;

        let text: String = parsed
            .content
            .into_iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");

        anyhow::ensure!(
            !text.is_empty(),
            "anthropic response contained no text content: {body}"
        );
        Ok(text)
    }
}
