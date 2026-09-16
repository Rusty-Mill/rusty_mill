use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use skillopt_core::{ChatBackend, Message, Role};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Default HTTP timeout for a single `chat` call. Chat completions can
/// legitimately take tens of seconds to generate, so this is generous
/// rather than tight -- it exists to recover from a stalled or hostile
/// endpoint instead of hanging `Engine::train` forever, matching the
/// `DEFAULT_TIMEOUT` convention `aisf_stage`/`claude_cli` already use for
/// the same purpose on their own (subprocess-based) request paths.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

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
    timeout: Duration,
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
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Overrides the default 120s HTTP timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
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
            .post(format!("{}/chat/completions", self.base_url))
            .timeout(self.timeout);
        if let Some(api_key) = &self.api_key {
            req_builder = req_builder.bearer_auth(api_key);
        }
        let resp = req_builder.json(&req).send().await?;

        let (status, body) = crate::http::read_capped_body(resp).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn backend(base_url: String) -> OpenAiCompatBackend {
        OpenAiCompatBackend::new(None, Some(base_url), "test-model".into(), None, 64)
    }

    #[tokio::test]
    async fn chat_times_out_on_stalled_server_instead_of_hanging() {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            addr_tx
                .send(listener.local_addr().unwrap().to_string())
                .unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            std::thread::sleep(std::time::Duration::from_secs(2));
        });
        let addr = addr_rx.recv().unwrap();
        let backend = backend(format!("http://{addr}")).with_timeout(Duration::from_millis(200));

        let start = std::time::Instant::now();
        let err = backend.chat(&[Message::user("hi")]).await.unwrap_err();
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "chat should have timed out quickly instead of hanging, took {:?}",
            start.elapsed()
        );
        assert!(
            err.chain()
                .filter_map(|e| e.downcast_ref::<reqwest::Error>())
                .any(reqwest::Error::is_timeout),
            "expected a timeout error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn chat_rejects_oversized_response_instead_of_buffering_it() {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            addr_tx
                .send(listener.local_addr().unwrap().to_string())
                .unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let oversized_len = crate::http::MAX_RESPONSE_BODY_BYTES as u64 + 1;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {oversized_len}\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes());
        });
        let addr = addr_rx.recv().unwrap();

        let backend = backend(format!("http://{addr}")).with_timeout(Duration::from_secs(5));
        let err = backend.chat(&[Message::user("hi")]).await.unwrap_err();
        assert!(
            err.to_string().contains("cap"),
            "expected an over-cap rejection, got: {err}"
        );

        let _ = server.join();
    }
}
