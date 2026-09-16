//! A deterministic in-process [`McpClient`] for offline tests — the MCP analog
//! of `FakeLanguageModel`. It serves a fixed tool list and canned results, and
//! can simulate a server crash (calls error) until `reconnect()` restores it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use serde_json::Value;

use crate::{McpClient, McpError, McpToolInfo};

/// A scripted MCP server.
pub struct FakeMcpClient {
    tools: Vec<McpToolInfo>,
    results: HashMap<String, String>,
    crashed: AtomicBool,
}

impl FakeMcpClient {
    /// Build with `(tool_name, schema)` advertisements and `(tool_name, result)`
    /// canned responses.
    pub fn new(tools: Vec<(&str, Value)>, results: Vec<(&str, &str)>) -> Self {
        Self {
            tools: tools
                .into_iter()
                .map(|(name, schema)| McpToolInfo {
                    name: name.to_string(),
                    schema,
                })
                .collect(),
            results: results
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            crashed: AtomicBool::new(false),
        }
    }

    /// Simulate a server crash: subsequent calls error until `reconnect()`.
    pub fn crash(&self) {
        self.crashed.store(true, Ordering::SeqCst);
    }

    fn is_crashed(&self) -> bool {
        self.crashed.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl McpClient for FakeMcpClient {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        if self.is_crashed() {
            return Err(McpError::Transport("server crashed".into()));
        }
        Ok(self.tools.clone())
    }

    async fn call_tool(&self, name: &str, _args: Value) -> Result<String, McpError> {
        if self.is_crashed() {
            return Err(McpError::Transport("server crashed".into()));
        }
        self.results
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::CallFailed {
                tool: name.to_string(),
            })
    }

    async fn reconnect(&self) -> Result<(), McpError> {
        self.crashed.store(false, Ordering::SeqCst);
        Ok(())
    }
}
