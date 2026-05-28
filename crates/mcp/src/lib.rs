#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `mcp` — Model Context Protocol integration (ARCHITECTURE §4; PRD 07).
//!
//! Phase 12: the MCP **client** (manager + namespacing + `McpPolicy` +
//! `McpToolFn`→`ToolOutcome` + tool-return inspection + reconnect) over the
//! object-safe [`McpClient`] seam, with a deterministic in-process fake for
//! offline tests. The `rmcp`-backed transport adapters and the MCP **server**
//! mode (ADR-0029) sit behind the `rmcp` feature.

mod config;
mod endpoint;
mod inspect;
mod manager;
mod policy;
mod tool;

#[cfg(any(test, feature = "fake"))]
pub mod fake;
#[cfg(feature = "rmcp")]
mod transport;

pub use config::{load_mcp_config, McpConfig, ServerSpec, Transport};
pub use endpoint::{require_tls_for_non_loopback, resolve_bearer_token};
pub use inspect::{DefaultInspector, Inspection, ReturnInspector};
pub use manager::McpManager;
pub use policy::McpPolicy;
pub use tool::{McpToolDescriptor, McpToolFn};
#[cfg(feature = "rmcp")]
pub use transport::{client_from_spec, HttpMcpClient, StdioMcpClient};

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
    /// The call exceeded its deadline.
    #[error("mcp call timed out")]
    Timeout,
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
    /// The server's own (un-namespaced) tool name. The manager namespaces it.
    pub name: String,
    /// JSON schema advertised to the model.
    pub schema: Value,
}

/// Client over one external MCP server. Object-safe seam (ADR-0024); stored as
/// `Arc<dyn McpClient>`. The `rmcp`-backed adapters implement it (ADR-0029); a
/// `FakeMcpClient` drives offline tests.
#[async_trait]
pub trait McpClient: Send + Sync {
    /// Enumerate the server's tools (un-namespaced names).
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError>;
    /// Invoke a tool by its un-namespaced name; returns the result string.
    async fn call_tool(&self, name: &str, args: Value) -> Result<String, McpError>;
    /// Re-establish the transport after a crash. Default is a no-op (a fresh
    /// client needs no reconnect); transports override it.
    async fn reconnect(&self) -> Result<(), McpError> {
        Ok(())
    }
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
