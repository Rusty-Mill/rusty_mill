//! `McpPolicy` (PRD 07): server-level allowlist + fully-qualified tool blocklist
//! over `mcp__<server>__<tool>` names. Non-MCP tools pass through untouched, so
//! it composes in the same `PolicyChain` as the built-in policies. Vetting still
//! happens before dispatch (ADR-0007).

use async_trait::async_trait;
use rk_constrain::{Policy, PolicyError};
use serde_json::Value;

/// Gates MCP tool calls by server allowlist and tool blocklist.
pub struct McpPolicy {
    /// `None` = all servers allowed; `Some` = only these server names.
    allowed_servers: Option<Vec<String>>,
    /// Fully-qualified `mcp__server__tool` names to block.
    blocked_tools: Vec<String>,
}

impl McpPolicy {
    /// Allow every MCP server (the default), block nothing.
    pub fn allow_all() -> Self {
        Self {
            allowed_servers: None,
            blocked_tools: Vec::new(),
        }
    }

    /// Restrict to `servers` (an empty list blocks all MCP tools).
    pub fn allow_servers(servers: Vec<String>) -> Self {
        Self {
            allowed_servers: Some(servers),
            blocked_tools: Vec::new(),
        }
    }

    /// Block specific fully-qualified tool names.
    pub fn block_tools(mut self, tools: Vec<String>) -> Self {
        self.blocked_tools = tools;
        self
    }

    /// The server segment of a `mcp__<server>__<tool>` name.
    fn server_of(name: &str) -> Option<&str> {
        name.strip_prefix("mcp__")
            .and_then(|rest| rest.split_once("__"))
            .map(|(server, _)| server)
    }
}

#[async_trait]
impl Policy for McpPolicy {
    async fn before_tool(&self, name: &str, _args: &Value) -> Result<(), PolicyError> {
        let Some(server) = Self::server_of(name) else {
            return Ok(()); // not an MCP tool
        };
        if self.blocked_tools.iter().any(|t| t == name) {
            return Err(PolicyError::ModeForbidden {
                mode: "mcp_blocked",
                tool: name.to_string(),
            });
        }
        if let Some(allowed) = &self.allowed_servers {
            if !allowed.iter().any(|s| s == server) {
                return Err(PolicyError::ModeForbidden {
                    mode: "mcp_server_not_allowed",
                    tool: name.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn non_mcp_tools_pass() {
        let p = McpPolicy::allow_servers(vec![]);
        assert!(p.before_tool("read_file", &json!({})).await.is_ok());
    }

    #[tokio::test]
    async fn server_allowlist_gates() {
        let p = McpPolicy::allow_servers(vec!["filesystem".into()]);
        assert!(p
            .before_tool("mcp__filesystem__read_file", &json!({}))
            .await
            .is_ok());
        assert!(p.before_tool("mcp__other__do", &json!({})).await.is_err());
    }

    #[tokio::test]
    async fn tool_blocklist_gates() {
        let p = McpPolicy::allow_all().block_tools(vec!["mcp__fs__rm".into()]);
        assert!(p.before_tool("mcp__fs__rm", &json!({})).await.is_err());
        assert!(p.before_tool("mcp__fs__ls", &json!({})).await.is_ok());
    }
}
