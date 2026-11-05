//! The MCP client the suites drive the gateway with: `rusty_mcp_client_native`
//! over Streamable HTTP.

#![allow(dead_code, reason = "each suite uses the part of this client it needs")]

use std::time::Duration;

use rusty_mcp_client_native::proto::{ContentBlock, ProtocolVersion};
use rusty_mcp_client_native::{AsyncClient, ClientConfig, ClientError, HttpTransport, NoHandler};

pub use rusty_mcp_client_native::HttpConfig;

pub use rusty_mcp_client_native::proto;

/// A client connected through the gateway.
pub type Client = AsyncClient<HttpTransport, NoHandler>;

/// How long the handshake may take.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// Connects to the MCP endpoint at `url` and shakes hands.
pub async fn connect(url: impl Into<String>) -> Result<Client, ClientError> {
    connect_with(HttpConfig::new(url)).await
}

/// [`connect`], with the transport configured by the caller (a bearer token,
/// extra headers).
pub async fn connect_with(config: HttpConfig) -> Result<Client, ClientError> {
    handshake(config, client_config()).await
}

/// [`connect_with`], speaking only the classic `2025-11-25` revision: an
/// `initialize` handshake and an `Mcp-Session-Id` session, where the default
/// client tries the stateless revision first.
pub async fn connect_classic(config: HttpConfig) -> Result<Client, ClientError> {
    let mut client = client_config();
    client.versions = vec![ProtocolVersion::new(ProtocolVersion::V_2025_11_25)];
    handshake(config, client).await
}

fn client_config() -> ClientConfig {
    ClientConfig::new("agentgateway-tests", "0.1.0")
}

async fn handshake(config: HttpConfig, client: ClientConfig) -> Result<Client, ClientError> {
    let transport = HttpTransport::new(config)?;
    AsyncClient::connect(transport, client, NoHandler, HANDSHAKE).await
}

/// The text blocks of `content`, joined.
pub fn text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The text of a prompt's messages, joined.
pub fn prompt_text(messages: &[proto::PromptMessage]) -> String {
    messages
        .iter()
        .map(|message| text(std::slice::from_ref(&message.content)))
        .collect()
}
