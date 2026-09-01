//! `rmcp`-backed transport adapters (ADR-0029), behind the `rmcp` feature.
//! `rmcp` owns JSON-RPC framing + lifecycle; this module is a thin
//! [`McpClient`] adapter — Rusty Keys keeps namespacing, policy, approval, and
//! return-inspection *above* it.
//!
//! Two transports ride the same [`McpClient`] seam:
//! - [`StdioMcpClient`] — a local subprocess speaking JSON-RPC over stdio.
//! - [`HttpMcpClient`] — a remote endpoint over **Streamable HTTP** (the
//!   successor to HTTP+SSE), carrying a bearer token via the `Authorization`
//!   header and refusing plaintext to non-loopback hosts ([`crate::endpoint`]).
//!   `rmcp`'s worker handles SSE keep-alive + automatic reconnect; `reconnect`
//!   re-establishes the session after a hard failure.
//!
//! Not exercisable in offline CI (they spawn/contact a real server); the
//! `FakeMcpClient` covers the manager/policy/registration paths deterministically
//! and [`crate::endpoint`] unit-tests the TLS/auth hardening.

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::model::CallToolRequestParam;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::endpoint::{require_tls_for_non_loopback, resolve_bearer_token};
use crate::{McpClient, McpError, McpToolInfo, ServerSpec, Transport};

type Running = RunningService<RoleClient, ()>;

/// Enumerate a connected service's tools (un-namespaced).
async fn list_tools_on(svc: &Running) -> Result<Vec<McpToolInfo>, McpError> {
    let result = svc
        .peer()
        .list_tools(None)
        .await
        .map_err(|e| McpError::Transport(e.to_string()))?;
    Ok(result
        .tools
        .into_iter()
        .map(|t| McpToolInfo {
            name: t.name.to_string(),
            schema: Value::Object((*t.input_schema).clone()),
        })
        .collect())
}

/// Invoke a tool on a connected service and join its text content blocks.
async fn call_tool_on(svc: &Running, name: &str, args: Value) -> Result<String, McpError> {
    let arguments = match args {
        Value::Object(map) => Some(map),
        Value::Null => None,
        other => {
            let mut m = serde_json::Map::new();
            m.insert("value".into(), other);
            Some(m)
        }
    };
    let result = svc
        .peer()
        .call_tool(CallToolRequestParam {
            name: name.to_string().into(),
            arguments,
        })
        .await
        .map_err(|_| McpError::CallFailed {
            tool: name.to_string(),
        })?;
    Ok(result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Build a connected client for one `mcp.toml` server spec, dispatching on its
/// transport. The SSE/HTTP path resolves the bearer token from the env var
/// named by `auth_token_env` and enforces TLS for non-loopback hosts.
pub async fn client_from_spec(spec: &ServerSpec) -> Result<Arc<dyn McpClient>, McpError> {
    match spec.transport {
        Transport::Stdio => {
            let command = spec.command.as_deref().ok_or_else(|| {
                McpError::Connect(format!("stdio server '{}' has no command", spec.name))
            })?;
            Ok(Arc::new(StdioMcpClient::connect(command, &spec.args).await?))
        }
        Transport::Sse => {
            let url = spec.url.as_deref().ok_or_else(|| {
                McpError::Connect(format!("sse server '{}' has no url", spec.name))
            })?;
            let token =
                resolve_bearer_token(spec.auth_token_env.as_deref(), |k| std::env::var(k).ok());
            Ok(Arc::new(HttpMcpClient::connect(url, token).await?))
        }
    }
}

/// A stdio (child-process) MCP client over `rmcp`.
pub struct StdioMcpClient {
    command: String,
    args: Vec<String>,
    running: Mutex<Option<Running>>,
}

impl StdioMcpClient {
    /// Spawn `command args…` as an MCP server and complete the handshake.
    pub async fn connect(command: &str, args: &[String]) -> Result<Self, McpError> {
        let client = Self {
            command: command.to_string(),
            args: args.to_vec(),
            running: Mutex::new(None),
        };
        client.spawn().await?;
        Ok(client)
    }

    async fn spawn(&self) -> Result<(), McpError> {
        let mut cmd = tokio::process::Command::new(&self.command);
        cmd.args(&self.args);
        let transport =
            TokioChildProcess::new(cmd).map_err(|e| McpError::Connect(e.to_string()))?;
        let running = ()
            .serve(transport)
            .await
            .map_err(|e| McpError::Connect(e.to_string()))?;
        *self.running.lock().await = Some(running);
        Ok(())
    }
}

#[async_trait]
impl McpClient for StdioMcpClient {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let guard = self.running.lock().await;
        let svc = guard
            .as_ref()
            .ok_or_else(|| McpError::Transport("not connected".into()))?;
        list_tools_on(svc).await
    }

    async fn call_tool(&self, name: &str, args: Value) -> Result<String, McpError> {
        let guard = self.running.lock().await;
        let svc = guard
            .as_ref()
            .ok_or_else(|| McpError::Transport("not connected".into()))?;
        call_tool_on(svc, name, args).await
    }

    async fn reconnect(&self) -> Result<(), McpError> {
        *self.running.lock().await = None;
        self.spawn().await
    }
}

/// A remote MCP client over **Streamable HTTP** (HTTP+SSE successor). Sends the
/// bearer token as `Authorization: Bearer <token>` and refuses plaintext to
/// non-loopback hosts. `rmcp` handles SSE keep-alive and auto-reconnect of the
/// event stream; [`McpClient::reconnect`] rebuilds the session after a hard
/// failure.
pub struct HttpMcpClient {
    url: String,
    auth_token: Option<String>,
    running: Mutex<Option<Running>>,
}

impl HttpMcpClient {
    /// Connect to `url` and complete the handshake. `auth_token` (if any) is the
    /// raw bearer token. Returns an error before any traffic if `url` would send
    /// plaintext to a non-loopback host.
    pub async fn connect(url: &str, auth_token: Option<String>) -> Result<Self, McpError> {
        require_tls_for_non_loopback(url)?;
        let client = Self {
            url: url.to_string(),
            auth_token,
            running: Mutex::new(None),
        };
        client.spawn().await?;
        Ok(client)
    }

    async fn spawn(&self) -> Result<(), McpError> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url.clone());
        if let Some(token) = &self.auth_token {
            config = config.auth_header(token.clone());
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        let running = ()
            .serve(transport)
            .await
            .map_err(|e| McpError::Connect(e.to_string()))?;
        *self.running.lock().await = Some(running);
        Ok(())
    }
}

#[async_trait]
impl McpClient for HttpMcpClient {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let guard = self.running.lock().await;
        let svc = guard
            .as_ref()
            .ok_or_else(|| McpError::Transport("not connected".into()))?;
        list_tools_on(svc).await
    }

    async fn call_tool(&self, name: &str, args: Value) -> Result<String, McpError> {
        let guard = self.running.lock().await;
        let svc = guard
            .as_ref()
            .ok_or_else(|| McpError::Transport("not connected".into()))?;
        call_tool_on(svc, name, args).await
    }

    async fn reconnect(&self) -> Result<(), McpError> {
        *self.running.lock().await = None;
        self.spawn().await
    }
}
