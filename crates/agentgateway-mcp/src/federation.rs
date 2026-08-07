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
use std::time::Duration;

use agentgateway_config::{McpAuthorization, McpBackend, McpGuardrails};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
};
use tokio::sync::RwLock;

use crate::{
    gate::{Authorization, GateError},
    guardrails::{CallContext, Guardrails, GuardrailsError, Outcome},
    mutating_client::HeaderOverride,
    naming::{Resolution, ToolNamer},
    rules::{RuleError, RuleSet, ToolCall},
    target::Target,
};

/// JSON-RPC method names the guardrail chain is keyed on.
const TOOLS_CALL: &str = "tools/call";
const TOOLS_LIST: &str = "tools/list";

/// The verified token's claims, carried on the HTTP request.
///
/// The gateway validates the token; `rules` needs what was in it. Passing the
/// claims through the request extensions keeps that one-way: this crate never
/// looks at a token, and nothing here can decide a request was authenticated.
#[derive(Debug, Clone)]
pub struct TokenClaims(pub serde_json::Value);

/// Failure to build a federation.
#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    /// A route policy's patterns did not compile.
    #[error(transparent)]
    Gate(#[from] GateError),

    /// A route policy's CEL rules did not compile.
    #[error(transparent)]
    Rules(#[from] RuleError),

    /// A guardrail processor could not be configured.
    #[error(transparent)]
    Guardrails(#[from] GuardrailsError),

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
    rules: RuleSet,
    guardrails: Guardrails,
    /// Budget for a single upstream call.
    ///
    /// This is the timeout that actually bounds a tool call. A route's
    /// `requestTimeout` cannot: the Streamable HTTP transport returns its SSE
    /// response headers immediately and streams the result afterwards, so by
    /// the time a tool starts running the response has already been produced.
    backend_timeout: Option<Duration>,
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
        guardrails: Option<&McpGuardrails>,
        backend_timeout: Option<Duration>,
        at: &str,
    ) -> Result<Self, FederationError> {
        let guardrails = match guardrails {
            Some(policy) => Guardrails::new(policy, &format!("{at}.mcpGuardrails"))?,
            None => Guardrails::default(),
        };

        let at_policy = format!("{at}.mcpAuthorization");
        let (authorization, rules) = match authorization {
            Some(policy) => (
                Authorization::new(policy, &at_policy)?,
                RuleSet::new(&policy.rules, &at_policy)?,
            ),
            None => (Authorization::default(), RuleSet::default()),
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
                rules,
                guardrails,
                backend_timeout,
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

    /// Every federated tool name known at startup.
    ///
    /// Used to bound metric label cardinality: anything outside this set is
    /// labelled `other`, so a client cannot mint unbounded time series by
    /// calling names that do not exist.
    pub fn tool_names(&self) -> Vec<String> {
        self.inner
            .index
            .try_read()
            .map(|index| index.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Rebuild the federated-name index, returning any collision warnings.
    async fn refresh_index(&self) -> Vec<String> {
        let mut index = HashMap::new();
        let mut per_target: Vec<(String, Vec<String>)> = Vec::new();

        for target in &self.inner.targets {
            // Startup warm-up, not a client call, so no processor was asked
            // and there is nothing to mutate.
            if let Some(tools) = self
                .list_with_timeout(target, &HeaderOverride::default())
                .await
            {
                let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
                for name in &names {
                    index.insert(
                        self.inner.namer.qualify(&target.name, name),
                        target.name.clone(),
                    );
                }
                per_target.push((target.name.clone(), names));
            }
        }

        *self.inner.index.write().await = index;

        self.inner.namer.collisions(
            per_target
                .iter()
                .map(|(name, tools)| (name.as_str(), tools.as_slice())),
        )
    }

    /// List a target's tools, giving up if it exceeds the backend budget.
    ///
    /// A target that hangs would otherwise hold up every `tools/list`, turning
    /// one sick server into a broken catalogue for all of them.
    async fn list_with_timeout(
        &self,
        target: &Target,
        headers: &HeaderOverride,
    ) -> Option<Vec<Tool>> {
        let listing = target.tools(headers);
        let result = match self.inner.backend_timeout {
            Some(budget) => match tokio::time::timeout(budget, listing).await {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!(
                        target = %target.name,
                        timeout_ms = budget.as_millis() as u64,
                        "listing tools exceeded the backend budget"
                    );
                    return None;
                }
            },
            None => listing.await,
        };

        match result {
            Ok(tools) => Some(tools),
            Err(err) => {
                tracing::warn!(target = %target.name, %err, "listing tools failed");
                None
            }
        }
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

/// The verified token's claims for this request, if the route validated one.
///
/// `rmcp` puts the HTTP request's [`http::request::Parts`] into the request
/// context, which is where the gateway leaves them.
fn claims(context: &RequestContext<RoleServer>) -> Option<&serde_json::Value> {
    context
        .extensions
        .get::<http::request::Parts>()?
        .extensions
        .get::<TokenClaims>()
        .map(|claims| &claims.0)
}

impl ServerHandler for Federation {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            rmcp::model::Implementation::new("rusty-agent-gateway", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let claims = claims(&context);
        let backends: Vec<String> = self
            .inner
            .targets
            .iter()
            .map(|target| target.name.clone())
            .collect();

        // `tools/list` fans out, so the request phase runs once for the whole
        // client call rather than once per target. It carries no params, so a
        // processor can refuse here but has nothing to rewrite -- filtering a
        // catalogue is response-phase work.
        let mut upstream_headers = HeaderOverride::default();
        if self.inner.guardrails.runs_request(TOOLS_LIST) {
            let decision = self
                .inner
                .guardrails
                .check_request(
                    CallContext {
                        method: TOOLS_LIST,
                        headers: request_headers(&context),
                        claims,
                    },
                    &backends,
                    None,
                )
                .await;

            if let Outcome::Reject {
                code,
                message,
                data,
            } = decision.outcome
            {
                return Err(mcp_error(code, message, data));
            }

            // `tools/list` fans out, so a header change applies to every
            // target's request -- there is one client call and several
            // upstream ones, and singling one out would be arbitrary.
            upstream_headers = decision.headers.into();
        }

        let mut tools: Vec<Tool> = Vec::new();
        let mut index = HashMap::new();

        for target in &self.inner.targets {
            let upstream = match self.list_with_timeout(target, &upstream_headers).await {
                Some(tools) => tools,
                // One unhealthy target must not blank the whole catalogue.
                None => continue,
            };

            for mut tool in upstream {
                let federated = self.inner.namer.qualify(&target.name, &tool.name);
                if !self.inner.authorization.permits(&federated) {
                    continue;
                }

                // The index is written from the caller-independent gate only.
                // `rules` can decide differently for different callers, and an
                // index rebuilt from one caller's view would delete the
                // routing another caller needs. Nothing is authorized by being
                // in it -- `call_tool` re-checks every gate.
                index.insert(federated.clone(), target.name.clone());

                if !self.inner.rules.permits(ToolCall {
                    target: &target.name,
                    tool: &tool.name,
                    claims,
                }) {
                    continue;
                }

                tool.name = federated.into();
                tools.push(tool);
            }
        }

        *self.inner.index.write().await = index;

        // The response phase sees the merged catalogue, after the gates have
        // had their say -- so a processor filtering the listing is refining
        // what the route already permits rather than widening it.
        let listing = ListToolsResult {
            tools,
            ..Default::default()
        };
        self.guard_response(TOOLS_LIST, &backends, &listing, &context)
            .await
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
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

        // Rules are evaluated here rather than on the federated name, because
        // they are written against the tool's own name and its target -- see
        // the module docs on `rules`. Like every other gate this runs on the
        // call, not only on the listing.
        if !self.inner.rules.permits(ToolCall {
            target: &target.name,
            tool: &tool,
            claims: claims(&context),
        }) {
            return Err(McpError::invalid_request(
                format!("tool `{federated}` is not permitted on this route"),
                None,
            ));
        }

        // The target's own filters gate the call too, for the same reason.
        if !target.filter.permits(&tool) {
            return Err(McpError::invalid_request(
                format!("tool `{federated}` is not exposed by this gateway"),
                None,
            ));
        }

        let mut params = request;
        params.name = tool.clone().into();

        // Guardrails run last of the gates: a processor is consulted only
        // about calls that were otherwise going to happen, and it sees the
        // unmuxed name the upstream will actually receive.
        let backends = vec![target.name.clone()];
        let mut upstream_headers = HeaderOverride::default();
        if self.inner.guardrails.runs_request(TOOLS_CALL) {
            let encoded = serde_json::to_vec(&params).unwrap_or_default();
            let decision = self
                .inner
                .guardrails
                .check_request(
                    CallContext {
                        method: TOOLS_CALL,
                        headers: request_headers(&context),
                        claims: claims(&context),
                    },
                    &backends,
                    Some(&encoded),
                )
                .await;

            match decision.outcome {
                Outcome::Pass => {}
                Outcome::Mutated(body) => match serde_json::from_slice(&body) {
                    Ok(rewritten) => params = rewritten,
                    // A processor that returns something unusable is a
                    // processor that failed, so it takes the same path as one
                    // that could not be reached rather than being ignored.
                    Err(err) => {
                        tracing::warn!(%err, "a guardrail rewrote tools/call into something unusable");
                        return Err(McpError::internal_error(
                            "mcpGuardrails returned an unusable request".to_string(),
                            None,
                        ));
                    }
                },
                Outcome::Reject {
                    code,
                    message,
                    data,
                } => return Err(mcp_error(code, message, data)),
            }

            upstream_headers = decision.headers.into();
        }

        let call = target.call(params, &upstream_headers);
        let result = match self.inner.backend_timeout {
            Some(budget) => match tokio::time::timeout(budget, call).await {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!(
                        target = %target.name,
                        tool = %federated,
                        timeout_ms = budget.as_millis() as u64,
                        "tool call exceeded the backend budget"
                    );
                    // An error the caller can read, not a protocol error: the
                    // request was well-formed and the model deserves to be
                    // told the tool timed out rather than shown an opaque
                    // internal failure.
                    return Ok(CallToolResponse::Complete(CallToolResult::error(vec![
                        ContentBlock::text(format!(
                            "`{federated}` timed out after {}ms",
                            budget.as_millis()
                        )),
                    ])));
                }
            },
            None => call.await,
        };

        // An upstream failure skips the response phase. There is no result to
        // inspect, and asking a guardrail to approve a failure is not a
        // question it can answer.
        let result = match result {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(target = %target.name, tool = %federated, %err, "tool call failed");
                return Err(McpError::internal_error(
                    format!("calling `{federated}` failed: {err}"),
                    None,
                ));
            }
        };

        let result = match self
            .guard_response(TOOLS_CALL, &backends, &result, &context)
            .await
        {
            Ok(result) => result,
            Err(err) => return Err(err),
        };

        Ok(CallToolResponse::Complete(result))
    }
}

/// Run the response phase over a value, returning it possibly rewritten.
impl Federation {
    async fn guard_response<T>(
        &self,
        method: &str,
        backends: &[String],
        value: &T,
        context: &RequestContext<RoleServer>,
    ) -> Result<T, McpError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        if !self.inner.guardrails.runs_response(method) {
            return Ok(value.clone());
        }

        let encoded = serde_json::to_vec(value).unwrap_or_default();
        match self
            .inner
            .guardrails
            .check_response(
                CallContext {
                    method,
                    headers: request_headers(context),
                    claims: claims(context),
                },
                backends,
                &encoded,
            )
            .await
        {
            Outcome::Pass => Ok(value.clone()),
            Outcome::Mutated(body) => serde_json::from_slice(&body).map_err(|err| {
                tracing::warn!(method, %err, "a guardrail rewrote a result into something unusable");
                McpError::internal_error(
                    "mcpGuardrails returned an unusable result".to_string(),
                    None,
                )
            }),
            Outcome::Reject {
                code,
                message,
                data,
            } => Err(mcp_error(code, message, data)),
        }
    }
}

/// The HTTP headers carrying this MCP call, or an empty map for stdio.
fn request_headers(context: &RequestContext<RoleServer>) -> &http::HeaderMap {
    static EMPTY: std::sync::LazyLock<http::HeaderMap> =
        std::sync::LazyLock::new(http::HeaderMap::new);
    context
        .extensions
        .get::<http::request::Parts>()
        .map(|parts| &parts.headers)
        .unwrap_or(&EMPTY)
}

/// Build a JSON-RPC error from a guardrail's refusal.
fn mcp_error(code: i32, message: String, data: Option<serde_json::Value>) -> McpError {
    McpError {
        code: rmcp::model::ErrorCode(code),
        message: message.into(),
        data,
    }
}
