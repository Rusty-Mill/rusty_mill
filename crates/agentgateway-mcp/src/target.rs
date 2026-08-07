//! Connections to upstream MCP servers.
//!
//! A target is dialled once at startup and the connection is shared by every
//! request the gateway serves. That matters most for `stdio` targets: spawning
//! `npx @modelcontextprotocol/server-everything` per request would cost far
//! more than the call itself, and each spawn would lose whatever state the
//! server had built up.

use agentgateway_config::{McpTarget, McpTargetKind};
use rmcp::{
    RoleClient, ServiceExt,
    service::RunningService,
    transport::{StreamableHttpClientTransport, TokioChildProcess},
};

use crate::gate::{GateError, TargetFilter};

/// Failure to bring up a target.
#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    /// The target's tool filters did not compile.
    #[error(transparent)]
    Gate(#[from] GateError),

    /// The subprocess could not be spawned.
    #[error("target `{name}`: spawning `{cmd}`: {source}")]
    Spawn {
        /// Target name.
        name: String,
        /// Command we tried to run.
        cmd: String,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// The MCP handshake failed.
    #[error("target `{name}`: MCP handshake failed: {source}")]
    Handshake {
        /// Target name.
        name: String,
        /// Underlying failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A transport this build cannot speak.
    #[error(
        "target `{name}`: the deprecated HTTP+SSE transport is not supported; \
         point the target at the server's Streamable HTTP endpoint with `mcp:` instead"
    )]
    UnsupportedTransport {
        /// Target name.
        name: String,
    },
}

/// A live connection to one upstream MCP server.
pub struct Target {
    /// Name used to qualify this target's tools.
    pub name: String,
    /// Which of its tools are federated.
    pub filter: TargetFilter,
    service: RunningService<RoleClient, ()>,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // RunningService is not Debug, and dumping a live transport into a log
        // line would not help anyone anyway.
        f.debug_struct("Target")
            .field("name", &self.name)
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}

impl Target {
    /// Dial a target and complete the MCP handshake.
    pub async fn connect(config: &McpTarget, at: &str) -> Result<Self, TargetError> {
        let filter = TargetFilter::new(&config.filters, at)?;
        let name = config.name.clone();

        let service = match &config.kind {
            McpTargetKind::Stdio(stdio) => {
                let cmd = stdio.cmd.clone();
                let args = stdio.args.clone();
                let env = stdio.env.clone();
                let command = process_wrap::tokio::CommandWrap::with_new(&cmd, move |command| {
                    command.args(&args);
                    for (key, value) in &env {
                        command.env(key, value);
                    }
                });
                let transport =
                    TokioChildProcess::new(command).map_err(|source| TargetError::Spawn {
                        name: name.clone(),
                        cmd: stdio.cmd.clone(),
                        source,
                    })?;
                ().serve(transport)
                    .await
                    .map_err(|source| TargetError::Handshake {
                        name: name.clone(),
                        source: Box::new(source),
                    })?
            }
            McpTargetKind::Mcp(http) => {
                let uri = format!("http://{}:{}{}", http.host, http.port, http.path);
                let transport = StreamableHttpClientTransport::from_uri(uri);
                ().serve(transport)
                    .await
                    .map_err(|source| TargetError::Handshake {
                        name: name.clone(),
                        source: Box::new(source),
                    })?
            }
            McpTargetKind::Sse(_) => {
                return Err(TargetError::UnsupportedTransport { name });
            }
        };

        Ok(Target {
            name,
            filter,
            service,
        })
    }

    /// The tools this target exports, after its filters.
    pub async fn tools(&self) -> Result<Vec<rmcp::model::Tool>, rmcp::service::ServiceError> {
        let tools = self.service.list_all_tools().await?;
        Ok(tools
            .into_iter()
            .filter(|tool| self.filter.permits(&tool.name))
            .collect())
    }

    /// Forward a tool call upstream.
    pub async fn call(
        &self,
        params: rmcp::model::CallToolRequestParams,
    ) -> Result<rmcp::model::CallToolResult, rmcp::service::ServiceError> {
        self.service.call_tool(params).await
    }

    /// Close the connection, terminating the subprocess for stdio targets.
    pub async fn shutdown(self) {
        if let Err(err) = self.service.cancel().await {
            tracing::warn!(target = %self.name, %err, "error shutting down MCP target");
        }
    }
}
