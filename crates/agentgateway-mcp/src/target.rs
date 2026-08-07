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
    model::{
        CallToolRequest, ClientRequest, GetExtensions, GetPromptRequest, GetPromptRequestParams,
        ListPromptsRequest, ListResourceTemplatesRequest, ListResourcesRequest, ListToolsRequest,
        ReadResourceRequest, ReadResourceRequestParams, ServerResult,
    },
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use crate::{
    gate::{GateError, TargetFilter},
    mutating_client::{HeaderOverride, MutatingClient},
};

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
    /// Whether this target speaks HTTP, and so has headers to mutate.
    ///
    /// A `stdio` target talks over a pipe. A guardrail's `headerMutation`
    /// aimed at one has nowhere to land, and is dropped rather than quietly
    /// appearing somewhere else.
    pub http: bool,
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
                // `MutatingClient` rather than a bare `reqwest::Client`, so a
                // guardrail's `headerMutation` can reach the outgoing request.
                let transport = StreamableHttpClientTransport::with_client(
                    MutatingClient::default(),
                    StreamableHttpClientTransportConfig::with_uri(uri),
                );
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
            http: matches!(config.kind, McpTargetKind::Mcp(_)),
            service,
        })
    }

    /// The tools this target exports, after its filters.
    ///
    /// `headers` are a guardrail's changes to the upstream HTTP request, and
    /// are ignored for a `stdio` target.
    pub async fn tools(
        &self,
        headers: &HeaderOverride,
    ) -> Result<Vec<rmcp::model::Tool>, rmcp::service::ServiceError> {
        let mut request = ClientRequest::ListToolsRequest(ListToolsRequest::default());
        self.attach(&mut request, headers);

        let tools = match self.service.send_request(request).await? {
            ServerResult::ListToolsResult(result) => result.tools,
            other => {
                tracing::warn!(target = %self.name, ?other, "unexpected result for tools/list");
                Vec::new()
            }
        };

        Ok(tools
            .into_iter()
            .filter(|tool| self.filter.permits(&tool.name))
            .collect())
    }

    /// Forward a tool call upstream.
    ///
    /// `headers` are a guardrail's changes to the upstream HTTP request, and
    /// are ignored for a `stdio` target.
    pub async fn call(
        &self,
        params: rmcp::model::CallToolRequestParams,
        headers: &HeaderOverride,
    ) -> Result<rmcp::model::CallToolResult, rmcp::service::ServiceError> {
        let mut request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        self.attach(&mut request, headers);

        match self.service.send_request(request).await? {
            ServerResult::CallToolResult(result) => Ok(result),
            other => {
                tracing::warn!(target = %self.name, ?other, "unexpected result for tools/call");
                Err(rmcp::service::ServiceError::UnexpectedResponse)
            }
        }
    }

    /// Whether this target advertised prompts in its handshake.
    ///
    /// Read from the capabilities the server sent when it connected, so asking
    /// costs nothing. A target that never advertised prompts is not asked for
    /// them: `prompts/list` against such a server is a method-not-found error,
    /// and one target's missing capability should not read as a fault.
    pub fn serves_prompts(&self) -> bool {
        self.service
            .peer_info()
            .is_some_and(|info| info.capabilities.prompts.is_some())
    }

    /// Whether this target advertised resources in its handshake.
    pub fn serves_resources(&self) -> bool {
        self.service
            .peer_info()
            .is_some_and(|info| info.capabilities.resources.is_some())
    }

    /// The prompts this target exports.
    ///
    /// Prompts carry no per-target `filters`: `filters` names tools, and
    /// widening it silently to prompts would change what existing configs mean.
    /// `mcpAuthorization.rules` is what gates prompts.
    pub async fn prompts(
        &self,
        headers: &HeaderOverride,
    ) -> Result<Vec<rmcp::model::Prompt>, rmcp::service::ServiceError> {
        let mut request = ClientRequest::ListPromptsRequest(ListPromptsRequest::default());
        self.attach(&mut request, headers);

        Ok(match self.service.send_request(request).await? {
            ServerResult::ListPromptsResult(result) => result.prompts,
            other => {
                tracing::warn!(target = %self.name, ?other, "unexpected result for prompts/list");
                Vec::new()
            }
        })
    }

    /// Fetch one prompt.
    pub async fn get_prompt(
        &self,
        params: GetPromptRequestParams,
        headers: &HeaderOverride,
    ) -> Result<rmcp::model::GetPromptResult, rmcp::service::ServiceError> {
        let mut request = ClientRequest::GetPromptRequest(GetPromptRequest::new(params));
        self.attach(&mut request, headers);

        match self.service.send_request(request).await? {
            ServerResult::GetPromptResult(result) => Ok(result),
            other => {
                tracing::warn!(target = %self.name, ?other, "unexpected result for prompts/get");
                Err(rmcp::service::ServiceError::UnexpectedResponse)
            }
        }
    }

    /// The resources this target exports.
    pub async fn resources(
        &self,
        headers: &HeaderOverride,
    ) -> Result<Vec<rmcp::model::Resource>, rmcp::service::ServiceError> {
        let mut request = ClientRequest::ListResourcesRequest(ListResourcesRequest::default());
        self.attach(&mut request, headers);

        Ok(match self.service.send_request(request).await? {
            ServerResult::ListResourcesResult(result) => result.resources,
            other => {
                tracing::warn!(target = %self.name, ?other, "unexpected result for resources/list");
                Vec::new()
            }
        })
    }

    /// The resource templates this target exports.
    pub async fn resource_templates(
        &self,
        headers: &HeaderOverride,
    ) -> Result<Vec<rmcp::model::ResourceTemplate>, rmcp::service::ServiceError> {
        let mut request =
            ClientRequest::ListResourceTemplatesRequest(ListResourceTemplatesRequest::default());
        self.attach(&mut request, headers);

        Ok(match self.service.send_request(request).await? {
            ServerResult::ListResourceTemplatesResult(result) => result.resource_templates,
            other => {
                tracing::warn!(
                    target = %self.name,
                    ?other,
                    "unexpected result for resources/templates/list"
                );
                Vec::new()
            }
        })
    }

    /// Read one resource.
    pub async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
        headers: &HeaderOverride,
    ) -> Result<rmcp::model::ReadResourceResult, rmcp::service::ServiceError> {
        let mut request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(params));
        self.attach(&mut request, headers);

        match self.service.send_request(request).await? {
            ServerResult::ReadResourceResult(result) => Ok(result),
            other => {
                tracing::warn!(target = %self.name, ?other, "unexpected result for resources/read");
                Err(rmcp::service::ServiceError::UnexpectedResponse)
            }
        }
    }

    /// Put a guardrail's header changes where the transport will find them.
    ///
    /// `rmcp` carries request extensions in memory down to the transport, so
    /// this is how a per-call header reaches a connection whose own headers
    /// were fixed when it was dialled.
    fn attach(&self, request: &mut ClientRequest, headers: &HeaderOverride) {
        if headers.is_empty() {
            return;
        }
        if !self.http {
            tracing::debug!(
                target = %self.name,
                "a guardrail asked to change headers on a stdio target; there are none"
            );
            return;
        }
        request.extensions_mut().insert(headers.clone());
    }

    /// Close the connection, terminating the subprocess for stdio targets.
    pub async fn shutdown(self) {
        if let Err(err) = self.service.cancel().await {
            tracing::warn!(target = %self.name, %err, "error shutting down MCP target");
        }
    }
}
