//! `rmcp`-backed transport adapters (ADR-0029), behind the `rmcp` feature.
//! `rmcp` owns JSON-RPC framing + lifecycle; this module is a thin
//! [`McpClient`] adapter — Rusty Keys keeps namespacing, policy, approval, and
//! return-inspection *above* it. Only the stdio (child-process) client is wired
//! here; SSE is the documented follow-on (it reuses the same `McpClient` seam).
//!
//! Not exercisable in offline CI (it spawns a real server); the `FakeMcpClient`
//! covers the manager/policy/registration paths deterministically.

use async_trait::async_trait;
use rmcp::model::CallToolRequestParam;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{McpClient, McpError, McpToolInfo};

/// A stdio (child-process) MCP client over `rmcp`.
pub struct StdioMcpClient {
    command: String,
    args: Vec<String>,
    running: Mutex<Option<RunningService<RoleClient, ()>>>,
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
        let running = ().serve(transport).await.map_err(|e| McpError::Connect(e.to_string()))?;
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

    async fn call_tool(&self, name: &str, args: Value) -> Result<String, McpError> {
        let guard = self.running.lock().await;
        let svc = guard
            .as_ref()
            .ok_or_else(|| McpError::Transport("not connected".into()))?;
        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => Some({
                let mut m = serde_json::Map::new();
                m.insert("value".into(), other);
                m
            }),
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
        // Concatenate the text content blocks into the model-facing result.
        let text = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(text)
    }

    async fn reconnect(&self) -> Result<(), McpError> {
        // Drop any prior service, then respawn a fresh subprocess.
        *self.running.lock().await = None;
        self.spawn().await
    }
}
