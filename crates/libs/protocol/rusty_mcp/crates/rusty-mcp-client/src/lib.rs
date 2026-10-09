//! A protocol-only MCP client, split out of `rusty-mcp` (which is the server
//! scaffold). Host applications own configuration files, policy checks,
//! connection pooling and tool dispatch.
//!
//! [`client::McpClient::connect`] resolves authentication and initializes a
//! connection; [`client::McpServerSpec`] describes it and
//! [`client::McpTransport`] selects the transport. Authentication
//! declarations live in [`client_auth`].

pub mod client;
pub mod client_auth;

pub use client::{McpClient, McpClientError, McpServerSpec, McpTransport};
pub use client_auth::{AuthError, McpAuth, McpAuthSecret};
