#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `mcp` — Model Context Protocol integration (ARCHITECTURE §4; PRD 07).
//!
//! **Phase-1 stub.** This crate declares the seam — the [`McpClient`] trait, the
//! [`McpError`] taxonomy, the `mcp__server__tool` namespacing convention, and the
//! [`McpServerConfig`] shape — so the DAG slot (`mcp → config, constrain, feed`)
//! and error composition exist from day one. The actual client/server land in
//! Phase 12 on the official `rmcp` SDK (ADR-0029); no transport is implemented
//! here yet.

use async_trait::async_trait;
use serde_json::Value;

/// Errors from the MCP layer (ADR-0023; error-handling §2). Composes downhill
/// (`config`, `constrain`, `feed`).
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// Could not connect to the server.
    #[error("mcp connect failed: {0}")]
    Connect(String),
    /// Transport-level failure (stdio/SSE).
    #[error("mcp transport error: {0}")]
    Transport(String),
    /// JSON-RPC protocol error with a code.
    #[error("mcp protocol error {code}")]
    Protocol {
        /// JSON-RPC error code.
        code: i64,
    },
    /// A tool call returned a failure.
    #[error("mcp tool {tool} call failed")]
    CallFailed {
        /// The (un-namespaced) tool name.
        tool: String,
    },
    /// A policy blocked the call.
    #[error(transparent)]
    Policy(#[from] rk_constrain::PolicyError),
    /// A wrapped tool-dispatch error.
    #[error(transparent)]
    Tool(#[from] rk_feed::ToolError),
    /// A configuration error.
    #[error(transparent)]
    Config(#[from] rk_config::ConfigError),
}

/// How to reach an external MCP server (PRD 07; `mcp.toml`).
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Logical server name; the namespace in `mcp__<server>__<tool>`.
    pub name: String,
    /// Transport endpoint (a command line for stdio, or a URL for SSE).
    pub endpoint: String,
}

/// A tool exposed by an external MCP server.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    /// Namespaced tool id (`mcp__<server>__<tool>`).
    pub name: String,
    /// JSON schema advertised to the model.
    pub schema: Value,
}

/// Client over one external MCP server. Object-safe seam (ADR-0024); stored as
/// `Arc<dyn McpClient>`. Phase 12 implements it on `rmcp`.
#[async_trait]
pub trait McpClient: Send + Sync {
    /// Enumerate the server's tools (namespaced).
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError>;
    /// Invoke a namespaced tool; returns the model-facing result string.
    async fn call_tool(&self, name: &str, args: Value) -> Result<String, McpError>;
}

/// The `mcp__<server>__<tool>` namespacing convention (coding-standards §3).
pub fn namespaced(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespacing_matches_convention() {
        assert_eq!(
            namespaced("filesystem", "read_file"),
            "mcp__filesystem__read_file"
        );
    }
}
