//! Proxies tools from other, already-running MCP servers -- Direction B
//! ("rusty_provider as an MCP gateway") from the design doc.
//!
//! `rusty_mcp` only covers the server side of MCP (its `client` feature is
//! dev-dependency-only), so this module talks to `rmcp`'s client API
//! directly: spawning stdio subprocesses via `TokioChildProcess`, or
//! connecting to Streamable HTTP endpoints via `StreamableHttpClientTransport`.
//!
//! A connection that fails at startup is logged and skipped -- same
//! soft-fail convention as `[jwt]`/`[webhook]`/`[persistence]` elsewhere in
//! this codebase -- rather than a hard failure of the whole server.
//! Reconnect-with-backoff for a connection that drops later is out of scope
//! for this first cut; a dead connection just starts failing its calls,
//! which the client sees as an ordinary tool error.

use std::collections::HashMap;

use rmcp::model::{CallToolRequestParams, CallToolResponse, JsonObject, Tool};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use tokio::sync::RwLock;

use rp_router::{McpUpstreamConfig, McpUpstreamTransport};

/// One connected upstream MCP server, keyed by its configured name.
type Connections = HashMap<String, RunningService<RoleClient, ()>>;

/// Aggregates tools from every configured upstream MCP server behind one
/// name-prefixed tool namespace (`"{upstream}/{tool}"`).
pub struct McpGateway {
    connections: RwLock<Connections>,
}

impl McpGateway {
    /// A gateway with no upstreams configured.
    pub fn empty() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }

    /// Connect to every configured upstream, skipping (with a warning) any
    /// that fails.
    pub async fn connect(upstreams: &[McpUpstreamConfig]) -> Self {
        let mut connections = HashMap::new();
        for upstream in upstreams {
            match connect_one(upstream).await {
                Ok(service) => {
                    tracing::info!(upstream = %upstream.name, "connected MCP upstream");
                    connections.insert(upstream.name.clone(), service);
                }
                Err(error) => {
                    tracing::warn!(
                        upstream = %upstream.name,
                        %error,
                        "failed to connect MCP upstream; its tools won't be available"
                    );
                }
            }
        }
        Self {
            connections: RwLock::new(connections),
        }
    }

    /// Every proxied tool across every connected upstream, each renamed to
    /// `"{upstream}/{tool}"`. An upstream whose `tools/list` call fails is
    /// logged and skipped for this call, rather than failing the whole
    /// listing.
    pub async fn list_tools(&self) -> Vec<Tool> {
        let connections = self.connections.read().await;
        let mut tools = Vec::new();
        for (name, service) in connections.iter() {
            match service.peer().list_tools(None).await {
                Ok(result) => {
                    for mut tool in result.tools {
                        tool.name = format!("{name}/{}", tool.name).into();
                        tools.push(tool);
                    }
                }
                Err(error) => {
                    tracing::warn!(upstream = %name, %error, "failed to list tools from MCP upstream");
                }
            }
        }
        tools
    }

    /// Forward a `tools/call` to `upstream`'s `tool`, verbatim.
    pub async fn call_tool(
        &self,
        upstream: &str,
        tool: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResponse, GatewayError> {
        let connections = self.connections.read().await;
        let service = connections
            .get(upstream)
            .ok_or_else(|| GatewayError::UnknownUpstream(upstream.to_string()))?;

        let mut params = CallToolRequestParams::new(tool.to_string());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        service
            .call_tool_once(params)
            .await
            .map_err(GatewayError::Service)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("no MCP upstream named '{0}' is connected")]
    UnknownUpstream(String),
    #[error(transparent)]
    Service(#[from] rmcp::ServiceError),
}

async fn connect_one(
    upstream: &McpUpstreamConfig,
) -> anyhow::Result<RunningService<RoleClient, ()>> {
    match &upstream.transport {
        McpUpstreamTransport::Stdio { command, args } => {
            let transport =
                TokioChildProcess::new(tokio::process::Command::new(command).configure(|c| {
                    c.args(args);
                }))?;
            Ok(().serve(transport).await?)
        }
        McpUpstreamTransport::Http {
            url,
            bearer_token_env,
        } => {
            let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
            if let Some(var) = bearer_token_env {
                let token = std::env::var(var).map_err(|_| {
                    anyhow::anyhow!("bearer_token_env '{var}' is not set in the environment")
                })?;
                config = config.auth_header(token);
            }
            let transport = StreamableHttpClientTransport::from_config(config);
            Ok(().serve(transport).await?)
        }
    }
}
