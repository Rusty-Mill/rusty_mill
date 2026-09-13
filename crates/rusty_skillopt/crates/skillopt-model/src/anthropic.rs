use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use skillopt_core::{ChatBackend, Message, Role};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Default HTTP timeout for a single `chat` call. Chat completions can
/// legitimately take tens of seconds to generate, so this is generous
/// rather than tight -- it exists to recover from a stalled or hostile
/// endpoint instead of hanging `Engine::train` forever, matching the
/// `DEFAULT_TIMEOUT` convention `aisf_stage`/`claude_cli` already use for
/// the same purpose on their own (subprocess-based) request paths.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

pub struct AnthropicBackend {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    temperature: Option<f32>,
    max_tokens: u32,
    timeout: Duration,
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
            .timeout(self.timeout)
            .send()
            .await?;

        let (status, body) = crate::http::read_capped_body(resp).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Accepts one connection, reads the request, then stalls forever
    /// without writing a response -- simulating a hung endpoint.
    fn stall_after_request() -> (
        std::sync::mpsc::Receiver<String>,
        std::thread::JoinHandle<()>,
    ) {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            addr_tx
                .send(listener.local_addr().unwrap().to_string())
                .unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            std::thread::sleep(std::time::Duration::from_secs(2));
        });
        (addr_rx, handle)
    }

    fn backend(base_url: String) -> AnthropicBackend {
        AnthropicBackend::new(
            "test-key".into(),
            Some(base_url),
            "test-model".into(),
            None,
            64,
        )
    }

    #[tokio::test]
    async fn chat_times_out_on_stalled_server_instead_of_hanging() {
        let (addr_rx, _server) = stall_after_request();
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
