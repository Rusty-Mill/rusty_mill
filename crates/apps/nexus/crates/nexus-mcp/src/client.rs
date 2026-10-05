//! Nexus compatibility facade for the reusable protocol-only MCP client.
//!
//! Nexus configuration and policy are adapted in [`crate::config`]; transport,
//! authentication, timeout, pagination, and shutdown behavior live in
//! `rusty_mcp`.

pub use rusty_mcp::client::{McpClient, McpClientError};
