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

use agentgateway_config::{HeaderModifier, McpAuthorization, McpBackend, McpGuardrails};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
        GetPromptRequestParams, GetPromptResponse, ListPromptsResult, ListResourceTemplatesResult,
        ListResourcesResult, ListToolsResult, PaginatedRequestParams, Prompt,
        ReadResourceRequestParams, ReadResourceResponse, Resource, ResourceContents,
        ResourceTemplate, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
};
use tokio::sync::RwLock;

use crate::{
    gate::{Authorization, GateError},
    guardrails::{Annotations, CallContext, Guardrails, GuardrailsError, Outcome},
    mutating_client::HeaderOverride,
    naming::{Resolution, ToolNamer},
    rules::{Call, RuleError, RuleSet, Subject},
    span,
    target::Target,
    transform::{Transform, TransformError},
};

/// JSON-RPC method names the guardrail chain is keyed on.
const TOOLS_CALL: &str = "tools/call";
const TOOLS_LIST: &str = "tools/list";
const PROMPTS_LIST: &str = "prompts/list";
const PROMPTS_GET: &str = "prompts/get";
const RESOURCES_LIST: &str = "resources/list";
const RESOURCES_TEMPLATES_LIST: &str = "resources/templates/list";
const RESOURCES_READ: &str = "resources/read";

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

    /// A route's header modifier did not compile.
    #[error(transparent)]
    Transform(#[from] TransformError),

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
    /// The route's `requestHeaderModifier`, applied to upstream calls.
    transform: Transform,
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
    /// The same, for prompts. Kept apart from the tool index because a target
    /// may export a prompt and a tool of the same name, and they route
    /// independently.
    prompt_index: RwLock<HashMap<String, String>>,
    /// The same, for resource URIs.
    resource_index: RwLock<HashMap<String, String>>,
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
        request_headers: Option<&HeaderModifier>,
        backend_timeout: Option<Duration>,
        at: &str,
    ) -> Result<Self, FederationError> {
        let transform = match request_headers {
            Some(modifier) => Transform::new(modifier, &format!("{at}.requestHeaderModifier"))?,
            None => Transform::default(),
        };
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
                transform,
                backend_timeout,
                targets,
                degraded,
                index: RwLock::new(HashMap::new()),
                prompt_index: RwLock::new(HashMap::new()),
                resource_index: RwLock::new(HashMap::new()),
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
            // No processor is asked about a warm-up -- there is no client
            // call to ask about -- but the route's own header modifier still
            // applies. An upstream that requires a static header would
            // otherwise reject the one request the gateway makes on its own
            // behalf. Templated values find no annotations here and drop,
            // which is the right reading: nothing classified this.
            let headers = self
                .transformed::<()>(HeaderOverride::default(), Annotations::default())
                .headers;

            if let Some(tools) = self.list_with_timeout(target, &headers).await {
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
        // Advertise a capability only when some target actually has it.
        // Claiming prompts the federation cannot serve would have clients
        // calling `prompts/list` to be told the method does not exist.
        // Filled in field by field rather than through the typestate builder,
        // whose chained methods cannot be applied conditionally.
        let mut capabilities = ServerCapabilities::default();
        capabilities.tools = Some(Default::default());
        capabilities.prompts = self
            .inner
            .targets
            .iter()
            .any(Target::serves_prompts)
            .then(Default::default);
        capabilities.resources = self
            .inner
            .targets
            .iter()
            .any(Target::serves_resources)
            .then(Default::default);

        ServerInfo::new(capabilities).with_server_info(rmcp::model::Implementation::new(
            "rusty-agent-gateway",
            env!("CARGO_PKG_VERSION"),
        ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let span = span::request(TOOLS_LIST, &context);

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
        let mut upstream_headers = self
            .transformed::<()>(HeaderOverride::default(), Annotations::default())
            .headers;
        if self.inner.guardrails.runs_request(TOOLS_LIST) {
            let decision = self
                .inner
                .guardrails
                .check_request(
                    CallContext {
                        method: TOOLS_LIST,
                        headers: request_headers(&context),
                        claims,
                        // A fanout has no single subject to name.
                        subject: None,
                        target: None,
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

            span::annotate(&span, &decision.annotations);

            // `tools/list` fans out, so a header change applies to every
            // target's request -- there is one client call and several
            // upstream ones, and singling one out would be arbitrary.
            upstream_headers = self
                .transformed::<()>(decision.headers.into(), decision.annotations)
                .headers;
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

                if !self.inner.rules.permits(Call {
                    target: &target.name,
                    subject: Subject::Tool(&tool.name),
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
        self.guard_response(TOOLS_LIST, &backends, &listing, None, &context)
            .await
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let federated = request.name.to_string();

        let span = span::request(TOOLS_CALL, &context);

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
        if !self.inner.rules.permits(Call {
            target: &target.name,
            subject: Subject::Tool(&tool),
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
        let mut upstream_headers = self
            .transformed::<()>(HeaderOverride::default(), Annotations::default())
            .headers;
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
                        subject: Some(Subject::Tool(&tool)),
                        target: Some(&target.name),
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

            span::annotate(&span, &decision.annotations);
            upstream_headers = self
                .transformed::<()>(decision.headers.into(), decision.annotations)
                .headers;
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
            .guard_response(
                TOOLS_CALL,
                &backends,
                &result,
                Some((Subject::Tool(&tool), &target.name)),
                &context,
            )
            .await
        {
            Ok(result) => result,
            Err(err) => return Err(err),
        };

        Ok(CallToolResponse::Complete(result))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let claims = claims(&context);
        let targets: Vec<&Target> = self
            .inner
            .targets
            .iter()
            .filter(|t| t.serves_prompts())
            .collect();
        let backends: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();

        let span = span::request(PROMPTS_LIST, &context);

        let guarded = self
            .guard_request(PROMPTS_LIST, &backends, None, None, &context)
            .await?;
        span::annotate(&span, &guarded.annotations);
        let headers = guarded.headers;

        let mut prompts: Vec<Prompt> = Vec::new();
        let mut index = HashMap::new();

        for target in targets {
            let Some(upstream) = self
                .with_timeout(target.prompts(&headers), target, "prompts")
                .await
            else {
                continue;
            };

            for mut prompt in upstream {
                let federated = self.inner.namer.qualify(&target.name, &prompt.name);

                // Indexed before the caller-dependent gate, for the same
                // reason as tools: `rules` can answer differently per caller,
                // and an index built from one caller's view would delete the
                // routing another caller needs.
                index.insert(federated.clone(), target.name.clone());

                if !self.inner.rules.permits(Call {
                    target: &target.name,
                    subject: Subject::Prompt(&prompt.name),
                    claims,
                }) {
                    continue;
                }

                prompt.name = federated;
                prompts.push(prompt);
            }
        }

        *self.inner.prompt_index.write().await = index;

        let listing = ListPromptsResult {
            prompts,
            ..Default::default()
        };
        self.guard_response(PROMPTS_LIST, &backends, &listing, None, &context)
            .await
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let federated = request.name.clone();

        let span = span::request(PROMPTS_GET, &context);

        let Some((target, name)) = self.route_prompt(&federated).await else {
            return Err(McpError::invalid_params(
                format!("unknown prompt `{federated}`"),
                None,
            ));
        };

        // Checked on the fetch, not only on the listing. Nothing stops a
        // client asking for a name it was never shown.
        self.permit(&target.name, Subject::Prompt(&name), &federated, &context)?;

        let mut params = request;
        params.name = name.clone();

        let backends = vec![target.name.clone()];
        let headers = self
            .guard_request_json(
                PROMPTS_GET,
                &backends,
                &params,
                Some((Subject::Prompt(&params.name), &target.name)),
                &context,
            )
            .await?;
        span::annotate(&span, &headers.annotations);
        let (params, headers) = (headers.body.unwrap_or(params), headers.headers);
        // What was actually fetched, which a request-phase rewrite may have
        // changed. The response phase should describe the result it is looking
        // at, not the name the client happened to ask for.
        let fetched = params.name.clone();

        let result = self
            .with_timeout(target.get_prompt(params, &headers), target, "prompts/get")
            .await
            .ok_or_else(|| {
                McpError::internal_error(format!("fetching `{federated}` failed"), None)
            })?;

        let result = self
            .guard_response(
                PROMPTS_GET,
                &backends,
                &result,
                Some((Subject::Prompt(&fetched), &target.name)),
                &context,
            )
            .await?;

        Ok(GetPromptResponse::Complete(result))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let claims = claims(&context);
        let targets: Vec<&Target> = self
            .inner
            .targets
            .iter()
            .filter(|t| t.serves_resources())
            .collect();
        let backends: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();

        let span = span::request(RESOURCES_LIST, &context);

        let guarded = self
            .guard_request(RESOURCES_LIST, &backends, None, None, &context)
            .await?;
        span::annotate(&span, &guarded.annotations);
        let headers = guarded.headers;

        let mut resources: Vec<Resource> = Vec::new();
        let mut index = HashMap::new();

        for target in targets {
            let Some(upstream) = self
                .with_timeout(target.resources(&headers), target, "resources")
                .await
            else {
                continue;
            };

            for mut resource in upstream {
                let federated = self.inner.namer.qualify_uri(&target.name, &resource.uri);
                index.insert(federated.clone(), target.name.clone());

                if !self.inner.rules.permits(Call {
                    target: &target.name,
                    subject: Subject::Resource(&resource.uri),
                    claims,
                }) {
                    continue;
                }

                resource.uri = federated;
                resources.push(resource);
            }
        }

        *self.inner.resource_index.write().await = index;

        let listing = ListResourcesResult {
            resources,
            ..Default::default()
        };
        self.guard_response(RESOURCES_LIST, &backends, &listing, None, &context)
            .await
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        let claims = claims(&context);
        let targets: Vec<&Target> = self
            .inner
            .targets
            .iter()
            .filter(|t| t.serves_resources())
            .collect();
        let backends: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();

        let span = span::request(RESOURCES_TEMPLATES_LIST, &context);

        let guarded = self
            .guard_request(RESOURCES_TEMPLATES_LIST, &backends, None, None, &context)
            .await?;
        span::annotate(&span, &guarded.annotations);
        let headers = guarded.headers;

        let mut templates: Vec<ResourceTemplate> = Vec::new();

        for target in targets {
            let Some(upstream) = self
                .with_timeout(
                    target.resource_templates(&headers),
                    target,
                    "resources/templates",
                )
                .await
            else {
                continue;
            };

            for mut template in upstream {
                // A template is gated on its `uriTemplate`, which is what
                // upstream matches too -- a rule that permits a template
                // permits the shape, and each concrete read is gated again on
                // its own URI when it arrives.
                if !self.inner.rules.permits(Call {
                    target: &target.name,
                    subject: Subject::Resource(&template.uri_template),
                    claims,
                }) {
                    continue;
                }

                template.uri_template = self
                    .inner
                    .namer
                    .qualify_uri(&target.name, &template.uri_template);
                templates.push(template);
            }
        }

        let listing = ListResourceTemplatesResult {
            resource_templates: templates,
            ..Default::default()
        };
        self.guard_response(
            RESOURCES_TEMPLATES_LIST,
            &backends,
            &listing,
            None,
            &context,
        )
        .await
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let federated = request.uri.clone();

        let span = span::request(RESOURCES_READ, &context);

        let Some((target, uri)) = self.route_resource(&federated).await else {
            return Err(McpError::invalid_params(
                format!("unknown resource `{federated}`"),
                None,
            ));
        };

        self.permit(&target.name, Subject::Resource(&uri), &federated, &context)?;

        let mut params = request;
        params.uri = uri.clone();

        let backends = vec![target.name.clone()];
        let headers = self
            .guard_request_json(
                RESOURCES_READ,
                &backends,
                &params,
                Some((Subject::Resource(&params.uri), &target.name)),
                &context,
            )
            .await?;
        span::annotate(&span, &headers.annotations);
        let (mut params, headers) = (headers.body.unwrap_or(params), headers.headers);

        // The upstream knows its own URI, never the federated one. A guardrail
        // that rewrote the params could have put the federated form back.
        params.uri = strip_prefix(&self.inner.namer, &target.name, params.uri);
        // What was actually read, which a request-phase rewrite may have
        // changed.
        let read = params.uri.clone();

        let mut result = self
            .with_timeout(
                target.read_resource(params, &headers),
                target,
                "resources/read",
            )
            .await
            .ok_or_else(|| {
                McpError::internal_error(format!("reading `{federated}` failed"), None)
            })?;

        // Contents come back carrying the target's own URIs, which no client
        // could read back to us. Re-qualify them so the round trip closes.
        for content in &mut result.contents {
            let uri = match content {
                ResourceContents::TextResourceContents { uri, .. }
                | ResourceContents::BlobResourceContents { uri, .. } => uri,
                // `ResourceContents` is non-exhaustive upstream. A variant this
                // build has not seen is passed through rather than guessed at.
                _ => continue,
            };
            *uri = self.inner.namer.qualify_uri(&target.name, uri);
        }

        let result = self
            .guard_response(
                RESOURCES_READ,
                &backends,
                &result,
                Some((Subject::Resource(&read), &target.name)),
                &context,
            )
            .await?;

        Ok(ReadResourceResponse::Complete(result))
    }
}

/// Drop a target's own prefix from a URI, if it carries one.
fn strip_prefix(namer: &ToolNamer, target: &str, uri: String) -> String {
    match namer.resolve_uri(&uri) {
        Resolution::Qualified {
            target: owner,
            tool,
        } if owner == target => tool.to_string(),
        _ => uri,
    }
}

/// A guardrail's request-phase answer, ready to use.
struct Guarded<T> {
    /// The rewritten body, when a processor sent one.
    body: Option<T>,
    /// Header changes for the upstream call.
    headers: HeaderOverride,
    /// Values to put on this request's span.
    annotations: Annotations,
}

impl Federation {
    /// Route a federated prompt name to its target and the target's own name.
    async fn route_prompt(&self, federated: &str) -> Option<(&Target, String)> {
        match self.inner.namer.resolve(federated) {
            Resolution::Qualified { target, tool } => {
                let name = tool.to_string();
                self.target(target).map(|t| (t, name))
            }
            Resolution::Unqualified(name) => {
                let owner = self.inner.prompt_index.read().await.get(name).cloned()?;
                self.target(&owner).map(|t| (t, name.to_string()))
            }
        }
    }

    /// Route a federated resource URI to its target and the target's own URI.
    async fn route_resource(&self, federated: &str) -> Option<(&Target, String)> {
        match self.inner.namer.resolve_uri(federated) {
            Resolution::Qualified { target, tool } => {
                let uri = tool.to_string();
                self.target(target).map(|t| (t, uri))
            }
            Resolution::Unqualified(uri) => {
                let owner = self.inner.resource_index.read().await.get(uri).cloned()?;
                self.target(&owner).map(|t| (t, uri.to_string()))
            }
        }
    }

    /// Apply the route's rules to one subject, or produce the refusal.
    fn permit(
        &self,
        target: &str,
        subject: Subject<'_>,
        federated: &str,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        if self.inner.rules.permits(Call {
            target,
            subject,
            claims: claims(context),
        }) {
            return Ok(());
        }
        Err(McpError::invalid_request(
            format!(
                "{} `{federated}` is not permitted on this route",
                subject.noun()
            ),
            None,
        ))
    }

    /// Bound an upstream call by the backend budget, logging what gave out.
    async fn with_timeout<T, E: std::fmt::Display>(
        &self,
        call: impl std::future::Future<Output = Result<T, E>>,
        target: &Target,
        what: &str,
    ) -> Option<T> {
        let result = match self.inner.backend_timeout {
            Some(budget) => match tokio::time::timeout(budget, call).await {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!(
                        target = %target.name,
                        what,
                        timeout_ms = budget.as_millis() as u64,
                        "an upstream call exceeded the backend budget"
                    );
                    return None;
                }
            },
            None => call.await,
        };

        match result {
            Ok(value) => Some(value),
            Err(err) => {
                // One unhealthy target must not blank the whole listing.
                tracing::warn!(target = %target.name, what, %err, "an upstream call failed");
                None
            }
        }
    }

    /// Run the request phase for a method that carries no params.
    async fn guard_request(
        &self,
        method: &str,
        backends: &[String],
        params: Option<&[u8]>,
        about: Option<(Subject<'_>, &str)>,
        context: &RequestContext<RoleServer>,
    ) -> Result<Guarded<()>, McpError> {
        // Still runs when no processor is keyed on this method: a route may
        // set a static header without any guardrail at all.
        if !self.inner.guardrails.runs_request(method) {
            return Ok(self.transformed(HeaderOverride::default(), Annotations::default()));
        }

        let decision = self
            .inner
            .guardrails
            .check_request(
                CallContext {
                    method,
                    headers: request_headers(context),
                    claims: claims(context),
                    subject: about.map(|(subject, _)| subject),
                    target: about.map(|(_, target)| target),
                },
                backends,
                params,
            )
            .await;

        match decision.outcome {
            Outcome::Reject {
                code,
                message,
                data,
            } => Err(mcp_error(code, message, data)),
            _ => Ok(self.transformed(decision.headers.into(), decision.annotations)),
        }
    }

    /// The same, for a method whose params a processor may rewrite.
    async fn guard_request_json<T>(
        &self,
        method: &str,
        backends: &[String],
        params: &T,
        about: Option<(Subject<'_>, &str)>,
        context: &RequestContext<RoleServer>,
    ) -> Result<Guarded<T>, McpError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        // Still runs when no processor is keyed on this method: a route may
        // set a static header without any guardrail at all.
        if !self.inner.guardrails.runs_request(method) {
            return Ok(self.transformed(HeaderOverride::default(), Annotations::default()));
        }

        let encoded = serde_json::to_vec(params).unwrap_or_default();
        let decision = self
            .inner
            .guardrails
            .check_request(
                CallContext {
                    method,
                    headers: request_headers(context),
                    claims: claims(context),
                    subject: about.map(|(subject, _)| subject),
                    target: about.map(|(_, target)| target),
                },
                backends,
                Some(&encoded),
            )
            .await;

        let body = match decision.outcome {
            Outcome::Pass => None,
            Outcome::Mutated(raw) => match serde_json::from_slice(&raw) {
                Ok(rewritten) => Some(rewritten),
                // A processor that returns something unusable is a processor
                // that failed, so it takes the same path as one that could not
                // be reached rather than being ignored.
                Err(err) => {
                    tracing::warn!(method, %err, "a guardrail rewrote a request into something unusable");
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
        };

        let mut guarded = self.transformed(decision.headers.into(), decision.annotations);
        guarded.body = body;
        Ok(guarded)
    }

    /// Fold the route's header modifier into a guardrail's changes.
    ///
    /// Runs last, so route configuration wins over a processor's runtime
    /// decision, and so its templates see everything the whole chain produced.
    fn transformed<T>(&self, mut headers: HeaderOverride, annotations: Annotations) -> Guarded<T> {
        if !self.inner.transform.is_empty() {
            self.inner.transform.apply(&mut headers, &annotations);
        }
        Guarded {
            body: None,
            headers,
            annotations,
        }
    }
}

/// Run the response phase over a value, returning it possibly rewritten.
impl Federation {
    async fn guard_response<T>(
        &self,
        method: &str,
        backends: &[String],
        value: &T,
        about: Option<(Subject<'_>, &str)>,
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
                    subject: about.map(|(subject, _)| subject),
                    target: about.map(|(_, target)| target),
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
