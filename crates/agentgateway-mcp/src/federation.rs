//! The federated MCP server.
//!
//! One [`Federation`] fronts several upstream targets and presents them as a
//! single MCP server: `tools/list` returns the union of their catalogues under
//! qualified names, and `tools/call` routes back to whichever target owns the
//! tool.
//!
//! A target that fails to come up does not take the gateway down. Five targets
//! behind one endpoint means five things that can be broken at any moment, and
//! refusing to serve the four healthy ones because the fifth is restarting is
//! not the trade a gateway should make. Failures are logged loudly at startup
//! and the federation reports them through [`Federation::degraded`].

use std::collections::HashMap;
use std::sync::Arc;

use agentgateway_config::{McpBackend, McpAuthorization};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
};
use tokio::sync::RwLock;

use crate::{
    gate::{Authorization, GateError},
    naming::{Resolution, ToolNamer},
    target::Target,
};

/// Failure to build a federation.
#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    /// A route policy's patterns did not compile.
    #[error(transparent)]
    Gate(#[from] GateError),

    /// Every target failed to come up, so there is nothing to serve.
    #[error("no MCP target could be reached; the federation would serve nothing")]
    NoTargets,
}

/// A set of upstream MCP servers presented as one.
#[derive(Clone)]
pub struct Federation {
    inner: Arc<Inner>,
}

struct Inner {
    namer: ToolNamer,
    authorization: Authorization,
    targets: Vec<Target>,
    degraded: Vec<String>,
    /// Federated name to target name. Only consulted in passthrough mode,
    /// where the name carries no target to resolve from.
    index: RwLock<HashMap<String, String>>,
}

impl std::fmt::Debug for Federation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Federation")
            .field("targets", &self.inner.targets)
            .field("degraded", &self.inner.degraded)
            .finish_non_exhaustive()
    }
}

impl Federation {
    /// Connect every target and build the federation.
    ///
    /// Returns an error only when no target at all could be reached; partial
    /// failures are recorded in [`Federation::degraded`].
    pub async fn connect(
        backend: &McpBackend,
        authorization: Option<&McpAuthorization>,
        at: &str,
    ) -> Result<Self, FederationError> {
        let authorization = match authorization {
            Some(policy) => Authorization::new(policy, &format!("{at}.mcpAuthorization"))?,
            None => Authorization::default(),
        };

        let mut targets = Vec::new();
        let mut degraded = Vec::new();
        for (i, config) in backend.targets.iter().enumerate() {
            match Target::connect(config, &format!("{at}.targets[{i}]")).await {
                Ok(target) => {
                    tracing::info!(target = %target.name, "MCP target connected");
                    targets.push(target);
                }
                Err(err) => {
                    tracing::error!(target = %config.name, %err, "MCP target unavailable");
                    degraded.push(err.to_string());
                }
            }
        }

        if targets.is_empty() {
            return Err(FederationError::NoTargets);
        }

        let namer = ToolNamer::new(
            backend.name_mode,
            targets.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
        );

        let federation = Federation {
            inner: Arc::new(Inner {
                namer,
                authorization,
                targets,
                degraded,
                index: RwLock::new(HashMap::new()),
            }),
        };

        // Warm the index so a passthrough-mode `tools/call` works before any
        // client has called `tools/list`, and so name collisions surface at
        // startup rather than on whichever request happens to hit them.
        for warning in federation.refresh_index().await {
            tracing::warn!("{warning}");
        }

        Ok(federation)
    }

    /// Targets that failed to come up, as human-readable reasons.
    pub fn degraded(&self) -> &[String] {
        &self.inner.degraded
    }

    /// Targets that are live.
    pub fn target_names(&self) -> impl Iterator<Item = &str> {
        self.inner.targets.iter().map(|t| t.name.as_str())
    }

    /// Rebuild the federated-name index, returning any collision warnings.
    async fn refresh_index(&self) -> Vec<String> {
        let mut index = HashMap::new();
        let mut per_target: Vec<(String, Vec<String>)> = Vec::new();

        for target in &self.inner.targets {
            match target.tools().await {
                Ok(tools) => {
                    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
                    for name in &names {
                        index.insert(
                            self.inner.namer.qualify(&target.name, name),
                            target.name.clone(),
                        );
                    }
                    per_target.push((target.name.clone(), names));
                }
                Err(err) => {
                    tracing::warn!(target = %target.name, %err, "listing tools failed");
                }
            }
        }

        *self.inner.index.write().await = index;

        self.inner.namer.collisions(
            per_target
                .iter()
                .map(|(name, tools)| (name.as_str(), tools.as_slice())),
        )
    }

    fn target(&self, name: &str) -> Option<&Target> {
        self.inner.targets.iter().find(|t| t.name == name)
    }

    /// Resolve a federated tool name to the target that owns it.
    async fn route(&self, federated: &str) -> Option<(&Target, String)> {
        match self.inner.namer.resolve(federated) {
            Resolution::Qualified { target, tool } => {
                let tool = tool.to_string();
                self.target(target).map(|t| (t, tool))
            }
            Resolution::Unqualified(name) => {
                let owner = self.inner.index.read().await.get(name).cloned()?;
                self.target(&owner).map(|t| (t, name.to_string()))
            }
        }
    }
}

impl ServerHandler for Federation {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "rusty-agent-gateway",
                env!("CARGO_PKG_VERSION"),
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools: Vec<Tool> = Vec::new();
        let mut index = HashMap::new();

        for target in &self.inner.targets {
            let upstream = match target.tools().await {
                Ok(tools) => tools,
                Err(err) => {
                    // One unhealthy target must not blank the whole catalogue.
                    tracing::warn!(target = %target.name, %err, "listing tools failed");
                    continue;
                }
            };

            for mut tool in upstream {
                let federated = self.inner.namer.qualify(&target.name, &tool.name);
                if !self.inner.authorization.permits(&federated) {
                    continue;
                }
                index.insert(federated.clone(), target.name.clone());
                tool.name = federated.into();
                tools.push(tool);
            }
        }

        *self.inner.index.write().await = index;

        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let federated = request.name.to_string();

        // Authorization is checked here, not only in list_tools. Nothing stops
        // a client from calling a name it was never shown, so filtering the
        // catalogue alone would leave every hidden tool callable.
        if !self.inner.authorization.permits(&federated) {
            return Err(McpError::invalid_request(
                format!("tool `{federated}` is not permitted on this route"),
                None,
            ));
        }

        let Some((target, tool)) = self.route(&federated).await else {
            return Err(McpError::invalid_params(
                format!("unknown tool `{federated}`"),
                None,
            ));
        };

        // The target's own filters gate the call too, for the same reason.
        if !target.filter.permits(&tool) {
            return Err(McpError::invalid_request(
                format!("tool `{federated}` is not exposed by this gateway"),
                None,
            ));
        }

        let mut params = request;
        params.name = tool.into();

        match target.call(params).await {
            Ok(result) => Ok(CallToolResponse::Complete(result)),
            Err(err) => {
                tracing::warn!(target = %target.name, tool = %federated, %err, "tool call failed");
                Err(McpError::internal_error(
                    format!("calling `{federated}` failed: {err}"),
                    None,
                ))
            }
        }
    }
}
