use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use skillopt_core::{ChatBackend, Message, Role};

/// A recent stable GA Azure OpenAI API version, used when config doesn't
/// override `api_version`. Azure requires this as a query param; there's no
/// meaningful "unversioned" default the way there is for plain OpenAI.
pub const DEFAULT_API_VERSION: &str = "2024-06-01";

/// Default HTTP timeout for a single `chat` call. Chat completions can
/// legitimately take tens of seconds to generate, so this is generous
/// rather than tight -- it exists to recover from a stalled or hostile
/// endpoint instead of hanging `Engine::train` forever, matching the
/// `DEFAULT_TIMEOUT` convention `aisf_stage`/`claude_cli` already use for
/// the same purpose on their own (subprocess-based) request paths.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Azure OpenAI's chat-completions API differs from plain OpenAI in two
/// ways `openai_compatible` doesn't accommodate: auth is an `api-key`
/// header (not `Authorization: Bearer`), and the URL encodes both the
/// resource endpoint and the deployment name rather than taking a model in
/// the request body - `{endpoint}/openai/deployments/{deployment}/chat/completions?api-version=...`.
pub struct AzureOpenAiBackend {
    client: reqwest::Client,
    api_key: String,
    /// The resource endpoint, e.g. `https://my-resource.openai.azure.com`
    /// (trailing slash tolerated).
    endpoint: String,
    /// The deployment name (reuses `BackendConfig::model` - Azure
    /// deployments are user-named, commonly matching the underlying model).
    deployment: String,
    api_version: String,
    temperature: Option<f32>,
    max_tokens: u32,
    timeout: Duration,
}

impl AzureOpenAiBackend {
    pub fn new(
        api_key: String,
        endpoint: String,
        deployment: String,
        api_version: Option<String>,
        temperature: Option<f32>,
        max_tokens: u32,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            deployment,
            api_version: api_version.unwrap_or_else(|| DEFAULT_API_VERSION.to_string()),
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
struct AzureMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct AzureRequest {
    messages: Vec<AzureMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct AzureResponse {
    choices: Vec<AzureChoice>,
}

#[derive(Deserialize)]
struct AzureChoice {
    message: AzureResponseMessage,
}

#[derive(Deserialize)]
struct AzureResponseMessage {
    content: Option<String>,
}

#[async_trait]
impl ChatBackend for AzureOpenAiBackend {
    fn name(&self) -> &str {
        "azure_openai"
    }

    async fn chat(&self, messages: &[Message]) -> anyhow::Result<String> {
        let azure_messages: Vec<AzureMessage> = messages
            .iter()
            .map(|m| AzureMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                content: m.content.clone(),
            })
            .collect();

        let req = AzureRequest {
            messages: azure_messages,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };

        let url = format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint, self.deployment, self.api_version
        );

        let resp = self
            .client
            .post(url)
            .header("api-key", &self.api_key)
            .json(&req)
            .timeout(self.timeout)
            .send()
            .await?;

        let (status, body) = crate::http::read_capped_body(resp).await?;
        anyhow::ensure!(
            status.is_success(),
            "azure openai API error ({status}): {body}"
        );

        let parsed: AzureResponse = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!("failed to parse azure openai response: {e}\nbody: {body}")
        })?;

        let text = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("azure openai response contained no choices: {body}"))?;

        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn capture_one_request(
        addr_tx: std::sync::mpsc::Sender<String>,
    ) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            addr_tx
                .send(listener.local_addr().unwrap().to_string())
                .unwrap();

            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let mut received = String::new();
            loop {
                let n = stream.read(&mut buf).unwrap();
                received.push_str(&String::from_utf8_lossy(&buf[..n]));
                if received.contains("\r\n\r\n") || n == 0 {
                    break;
                }
            }

            let body = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();

            received
        })
    }

    #[tokio::test]
    async fn sends_api_key_header_and_deployment_url_shape() {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let server = capture_one_request(addr_tx);
        let addr = addr_rx.recv().unwrap();

        let backend = AzureOpenAiBackend::new(
            "secret-key".into(),
            format!("http://{addr}"),
            "my-deployment".into(),
            Some("2024-06-01".into()),
            None,
            64,
        );
        backend.chat(&[Message::user("hi")]).await.unwrap();

        let request = server.join().unwrap();
        let lowered = request.to_lowercase();
        assert!(
            lowered.contains("api-key: secret-key"),
            "expected api-key header, got:\n{request}"
        );
        assert!(
            !lowered.contains("authorization:"),
            "should not send Authorization header, got:\n{request}"
        );
        assert!(
            request.contains(
                "/openai/deployments/my-deployment/chat/completions?api-version=2024-06-01"
            ),
            "expected Azure URL shape, got:\n{request}"
        );
    }

    #[tokio::test]
    async fn defaults_api_version_when_unset() {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let server = capture_one_request(addr_tx);
        let addr = addr_rx.recv().unwrap();

        let backend = AzureOpenAiBackend::new(
            "secret-key".into(),
            format!("http://{addr}"),
            "dep".into(),
            None,
            None,
            64,
        );
        backend.chat(&[Message::user("hi")]).await.unwrap();

        let request = server.join().unwrap();
        assert!(
            request.contains(&format!("api-version={DEFAULT_API_VERSION}")),
            "expected default api-version, got:\n{request}"
        );
    }

    #[tokio::test]
    async fn strips_trailing_slash_from_endpoint() {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let server = capture_one_request(addr_tx);
        let addr = addr_rx.recv().unwrap();

        let backend = AzureOpenAiBackend::new(
            "secret-key".into(),
            format!("http://{addr}/"),
            "dep".into(),
            None,
            None,
            64,
        );
        backend.chat(&[Message::user("hi")]).await.unwrap();

        let request = server.join().unwrap();
        assert!(
            !request.contains("//openai/deployments"),
            "trailing slash on endpoint should not produce a double slash, got:\n{request}"
        );
    }

    fn backend(endpoint: String) -> AzureOpenAiBackend {
        AzureOpenAiBackend::new("secret-key".into(), endpoint, "dep".into(), None, None, 64)
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
