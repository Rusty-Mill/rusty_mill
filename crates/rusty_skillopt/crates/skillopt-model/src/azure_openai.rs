use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use skillopt_core::{ChatBackend, Message, Role};

/// A recent stable GA Azure OpenAI API version, used when config doesn't
/// override `api_version`. Azure requires this as a query param; there's no
/// meaningful "unversioned" default the way there is for plain OpenAI.
pub const DEFAULT_API_VERSION: &str = "2024-06-01";

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
        }
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
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
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
}
