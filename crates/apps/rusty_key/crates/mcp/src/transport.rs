//! The MCP transport adapter (ADR-0029), behind the `rmcp` feature.
//! `rusty-mcp-client` owns connection setup (stdio spawn, Streamable HTTP,
//! auth headers, handshake timeout) and pagination; this module is a thin
//! [`McpClient`] adapter -- Rusty Keys keeps namespacing, policy, approval, and
//! return-inspection *above* it, plus the endpoint hardening in
//! [`crate::endpoint`] (no plaintext to non-loopback hosts, bearer token from
//! the env var `auth_token_env` names).
//!
//! Not exercisable in offline CI (it spawns/contacts a real server); the
//! `FakeMcpClient` covers the manager/policy/registration paths deterministically
//! and [`crate::endpoint`] unit-tests the TLS/auth hardening.

use std::sync::Arc;

use async_trait::async_trait;
use rusty_mcp_client::proto::ContentBlock;
use rusty_mcp_client::{McpClient as Connection, McpServerSpec, McpTransport};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::endpoint::{require_tls_for_non_loopback, resolve_bearer_token};
use crate::{McpClient, McpError, McpToolInfo, ServerSpec, Transport};

/// Build a connected client for one `mcp.toml` server spec, dispatching on its
/// transport. The SSE/HTTP path resolves the bearer token from the env var
/// named by `auth_token_env` and enforces TLS for non-loopback hosts.
pub async fn client_from_spec(spec: &ServerSpec) -> Result<Arc<dyn McpClient>, McpError> {
    let connection_spec = match spec.transport {
        Transport::Stdio => {
            let command = spec.command.as_deref().ok_or_else(|| {
                McpError::Connect(format!("stdio server '{}' has no command", spec.name))
            })?;
            McpServerSpec {
                transport: McpTransport::Stdio,
                command: command.to_string(),
                args: spec.args.clone(),
                ..McpServerSpec::default()
            }
        }
        Transport::Sse => {
            let url = spec.url.as_deref().ok_or_else(|| {
                McpError::Connect(format!("sse server '{}' has no url", spec.name))
            })?;
            require_tls_for_non_loopback(url)?;
            let token =
                resolve_bearer_token(spec.auth_token_env.as_deref(), |k| std::env::var(k).ok());
            McpServerSpec {
                transport: McpTransport::Http,
                url: Some(url.to_string()),
                auth_header: token,
                ..McpServerSpec::default()
            }
        }
    };
    Ok(Arc::new(
        RemoteMcpClient::connect(&spec.name, connection_spec).await?,
    ))
}

/// One external MCP server over `rusty-mcp-client`. Keeps its spec so
/// [`McpClient::reconnect`] can rebuild the connection after a hard failure.
pub struct RemoteMcpClient {
    name: String,
    spec: McpServerSpec,
    connection: Mutex<Option<Connection>>,
}

impl RemoteMcpClient {
    /// Connect to the server `spec` describes and complete the handshake.
    pub async fn connect(name: &str, spec: McpServerSpec) -> Result<Self, McpError> {
        let client = Self {
            name: name.to_string(),
            spec,
            connection: Mutex::new(None),
        };
        client.reconnect().await?;
        Ok(client)
    }
}

#[async_trait]
impl McpClient for RemoteMcpClient {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let guard = self.connection.lock().await;
        let connection = guard
            .as_ref()
            .ok_or_else(|| McpError::Transport("not connected".into()))?;
        let tools = connection
            .list_tools()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        Ok(tools
            .into_iter()
            .map(|t| McpToolInfo {
                name: t.name.to_string(),
                schema: serde_json::from_str(&t.input_schema.to_json_string())
                    .unwrap_or(Value::Null),
            })
            .collect())
    }

    async fn call_tool(&self, name: &str, args: Value) -> Result<String, McpError> {
        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => {
                let mut m = serde_json::Map::new();
                m.insert("value".into(), other);
                Some(m)
            }
        };
        let guard = self.connection.lock().await;
        let connection = guard
            .as_ref()
            .ok_or_else(|| McpError::Transport("not connected".into()))?;
        let result =
            connection
                .call_tool(name, arguments)
                .await
                .map_err(|_| McpError::CallFailed {
                    tool: name.to_string(),
                })?;
        Ok(result
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    async fn reconnect(&self) -> Result<(), McpError> {
        // Drop the old connection first: it closes the transport and kills a
        // stdio child before the replacement is spawned.
        *self.connection.lock().await = None;
        let connection = Connection::connect(&self.name, &self.spec)
            .await
            .map_err(|e| McpError::Connect(e.to_string()))?;
        *self.connection.lock().await = Some(connection);
        Ok(())
    }
}
