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

//!
//! # How it is served
//!
//! The federation is the three request-time sources of a `rusty_mcp_server`
//! ([`ToolSource`], [`PromptSource`], [`ResourceSource`]): the server asks it
//! for each listing and hands it each call, on a worker thread of its own. The
//! guardrail chain is async (it speaks gRPC), so the sources run it with
//! [`Handle::block_on`]; they must not be called from async code.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use agentgateway_config::{HeaderModifier, McpAuthorization, McpBackend, McpGuardrails};
use rusty_mcp_client_native::ClientError;
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{
    CallToolParams, CallToolResult, ContentBlock, ErrorCode, ErrorData, GetPromptParams,
    GetPromptResult, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, Prompt, ReadResourceParams, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate, Tool, Wire,
};
use rusty_mcp_server::{
    BuildError, CallContext as Call_, PromptSource, ResourceSource, Server, ToolSource,
};
use tokio::runtime::Handle;

use crate::{
    gate::{Authorization, GateError},
    guardrails::{Annotations, CallContext, Guardrails, GuardrailsError, Outcome},
    header_override::HeaderOverride,
    naming::{Resolution, ToolNamer},
    rules::{Call, RuleError, RuleSet, Subject},
    span,
    target::{Override, Target, TargetError},
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
    #[error("no MCP target could be reached; the federation would serve nothing: {0}")]
    NoTargets(String),

    /// The server could not be assembled from the federation.
    #[error("assembling the MCP server: {0}")]
    Build(#[from] BuildError),
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
    /// Runs the async guardrail chain for the worker threads that call us.
    runtime: Handle,
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

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Federation {
    /// Connect every target and build the federation.
    ///
    /// Returns an error only when no target at all could be reached; partial
    /// failures are recorded in [`Federation::degraded`]. Must be called from a
    /// tokio runtime, which the federation keeps to run its guardrail chain.
    #[allow(
        clippy::too_many_arguments,
        reason = "every one is a distinct piece of a route's configuration, and grouping them \
                  into a struct would only move the same list somewhere with a name that means \
                  less than the parameters do"
    )]
    pub async fn connect(
        backend: &McpBackend,
        authorization: Option<&McpAuthorization>,
        guardrails: Option<&McpGuardrails>,
        registry: &agentgateway_core::Registry,
        request_headers: Option<&HeaderModifier>,
        overrides: Vec<Override>,
        backend_timeout: Option<Duration>,
        at: &str,
    ) -> Result<Self, FederationError> {
        let transform = match request_headers {
            Some(modifier) => Transform::new(modifier, &format!("{at}.requestHeaderModifier"))?,
            None => Transform::default(),
        };
        let guardrails = match guardrails {
            Some(policy) => Guardrails::new(policy, registry, &format!("{at}.mcpGuardrails"))?,
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

        // Dialling blocks (a handshake, or spawning a child), so it happens
        // off the async threads.
        let dials: Vec<_> = backend
            .targets
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, config)| {
                // One override per target, in order: a path rewrite transforms
                // each target's own path, so they are not interchangeable.
                let over = overrides.get(i).cloned().unwrap_or_default();
                let place = format!("{at}.targets[{i}]");
                tokio::task::spawn_blocking(move || {
                    let result = Target::connect(&config, &over, backend_timeout, &place);
                    (config.name, result)
                })
            })
            .collect();

        let mut targets = Vec::new();
        let mut degraded = Vec::new();
        for dial in dials {
            match dial.await {
                Ok((name, Ok(target))) => {
                    tracing::info!(target = %name, "MCP target connected");
                    targets.push(target);
                }
                Ok((name, Err(err))) => {
                    tracing::error!(target = %name, %err, "MCP target unavailable");
                    degraded.push(err.to_string());
                }
                Err(err) => {
                    let err = TargetError::Handshake {
                        name: "?".to_owned(),
                        source: Box::new(err),
                    };
                    tracing::error!(%err, "MCP target unavailable");
                    degraded.push(err.to_string());
                }
            }
        }

        if targets.is_empty() {
            return Err(FederationError::NoTargets(degraded.join("; ")));
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
                runtime: Handle::current(),
                index: RwLock::new(HashMap::new()),
                prompt_index: RwLock::new(HashMap::new()),
                resource_index: RwLock::new(HashMap::new()),
            }),
        };

        // Warm the index so a passthrough-mode `tools/call` works before any
        // client has called `tools/list`, and so name collisions surface at
        // startup rather than on whichever request happens to hit them.
        let warm = federation.clone();
        let warnings = tokio::task::spawn_blocking(move || warm.refresh_index())
            .await
            .unwrap_or_default();
        for warning in warnings {
            tracing::warn!("{warning}");
        }

        Ok(federation)
    }

    /// The MCP server to mount: this federation as the source of every tool,
    /// and of prompts and resources when a target has them.
    ///
    /// # Errors
    /// [`FederationError::Build`] if the server cannot be assembled.
    pub fn server(&self) -> Result<Server, FederationError> {
        // Advertise a capability only when some target actually has it.
        // Claiming prompts the federation cannot serve would have clients
        // calling `prompts/list` to be told the method does not exist.
        let mut builder = Server::builder("rusty-agent-gateway", env!("CARGO_PKG_VERSION"))
            .tool_source(self.clone());
        if self.inner.targets.iter().any(Target::serves_prompts) {
            builder = builder.prompt_source(self.clone());
        }
        if self.inner.targets.iter().any(Target::serves_resources) {
            builder = builder.resource_source(self.clone());
        }
        Ok(builder.build()?)
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
    fn refresh_index(&self) -> Vec<String> {
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

            if let Some(tools) = self.list_with_timeout(target, &headers) {
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

        *write(&self.inner.index) = index;

        self.inner.namer.collisions(
            per_target
                .iter()
                .map(|(name, tools)| (name.as_str(), tools.as_slice())),
        )
    }

    /// List a target's tools. A target that fails, or exceeds the backend
    /// budget, would otherwise turn one sick server into a broken catalogue
    /// for all of them, so it contributes nothing instead.
    fn list_with_timeout(&self, target: &Target, headers: &HeaderOverride) -> Option<Vec<Tool>> {
        self.upstream(target, "tools", || target.tools(headers))
    }

    fn target(&self, name: &str) -> Option<&Target> {
        self.inner.targets.iter().find(|t| t.name == name)
    }

    /// Resolve a federated tool name to the target that owns it.
    fn route(&self, federated: &str) -> Option<(&Target, String)> {
        match self.inner.namer.resolve(federated) {
            Resolution::Qualified { target, tool } => {
                let tool = tool.to_string();
                self.target(target).map(|t| (t, tool))
            }
            Resolution::Unqualified(name) => {
                let owner = read(&self.inner.index).get(name).cloned()?;
                self.target(&owner).map(|t| (t, name.to_string()))
            }
        }
    }
}

fn error(code: ErrorCode, message: impl Into<String>) -> ErrorData {
    ErrorData::new(code, message)
}

/// What the call being served knows about its caller, in the forms the gate,
/// rules and guardrails take.
struct Who {
    headers: http::HeaderMap,
    claims: Option<serde_json::Value>,
}

impl Who {
    fn of(ctx: &Call_) -> Self {
        let caller = ctx.caller();
        let mut headers = http::HeaderMap::new();
        for (name, value) in &caller.headers {
            if let (Ok(name), Ok(value)) = (
                http::HeaderName::from_bytes(name.as_bytes()),
                http::HeaderValue::from_str(value),
            ) {
                headers.append(name, value);
            }
        }
        // The verified token's claims, as the route's auth left them.
        let claims = caller
            .principal
            .as_ref()
            .and_then(|p| serde_json::from_str(&p.to_json_string()).ok());
        Who { headers, claims }
    }

    fn claims(&self) -> Option<&serde_json::Value> {
        self.claims.as_ref()
    }
}

/// `_meta` as forwarded upstream: only the trace context. The client's own
/// revision, capabilities and progress token describe the downstream
/// connection and mean nothing to the upstream one.
fn forward_meta(meta: &Option<Value>) -> Option<Value> {
    let meta = meta.as_ref()?;
    let mut kept = Value::object();
    for key in ["traceparent", "tracestate", "baggage"] {
        if let Some(value) = meta.get(key) {
            kept.insert(key, value.clone());
        }
    }
    kept.as_object().is_some_and(|o| !o.is_empty()).then_some(kept)
}

/// A guardrail's JSON-RPC refusal.
fn refusal(code: i32, message: String, data: Option<serde_json::Value>) -> ErrorData {
    ErrorData {
        code: ErrorCode(code),
        message,
        data: data.and_then(|d| Value::from_json_str(&d.to_string()).ok()),
    }
}

/// `value` as the JSON bytes a guardrail is shown.
fn encode(value: &impl Wire) -> Vec<u8> {
    value.to_value().to_json_string().into_bytes()
}

/// A guardrail's rewrite, decoded.
fn decode<T: Wire>(body: &[u8]) -> Option<T> {
    let text = std::str::from_utf8(body).ok()?;
    T::from_value(&Value::from_json_str(text).ok()?).ok()
}

impl ToolSource for Federation {
    fn tools(&self) -> Vec<Tool> {
        Vec::new()
    }

    fn tools_for(&self, ctx: &Call_) -> Result<Vec<Tool>, ErrorData> {
        let span = span::request(TOOLS_LIST, ctx);
        let who = Who::of(ctx);
        let claims = who.claims();
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
            let decision = self.block(self.inner.guardrails.check_request(
                CallContext {
                    method: TOOLS_LIST,
                    headers: &who.headers,
                    claims,
                    // A fanout has no single subject to name.
                    subject: None,
                    target: None,
                },
                &backends,
                None,
            ));

            if let Outcome::Reject {
                code,
                message,
                data,
            } = decision.outcome
            {
                return Err(refusal(code, message, data));
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
            let Some(upstream) = self.list_with_timeout(target, &upstream_headers) else {
                // One unhealthy target must not blank the whole catalogue.
                continue;
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

                tool.name = federated;
                tools.push(tool);
            }
        }

        *write(&self.inner.index) = index;

        // The response phase sees the merged catalogue, after the gates have
        // had their say -- so a processor filtering the listing is refining
        // what the route already permits rather than widening it.
        let listing = ListToolsResult {
            tools,
            ..Default::default()
        };
        self.guard_response(TOOLS_LIST, &backends, &listing, None, &who)
            .map(|l| l.tools)
    }

    fn call(
        &self,
        ctx: &Call_,
        request: &CallToolParams,
    ) -> Option<Result<CallToolResult, ErrorData>> {
        Some(self.call_tool(ctx, request))
    }
}

impl Federation {
    fn call_tool(
        &self,
        ctx: &Call_,
        request: &CallToolParams,
    ) -> Result<CallToolResult, ErrorData> {
        let federated = request.name.clone();
        let span = span::request(TOOLS_CALL, ctx);
        let who = Who::of(ctx);

        // Authorization is checked here, not only in list_tools. Nothing stops
        // a client from calling a name it was never shown, so filtering the
        // catalogue alone would leave every hidden tool callable.
        if !self.inner.authorization.permits(&federated) {
            return Err(error(
                ErrorCode::INVALID_REQUEST,
                format!("tool `{federated}` is not permitted on this route"),
            ));
        }

        let Some((target, tool)) = self.route(&federated) else {
            return Err(error(
                ErrorCode::INVALID_PARAMS,
                format!("unknown tool `{federated}`"),
            ));
        };

        // Rules are evaluated here rather than on the federated name, because
        // they are written against the tool's own name and its target -- see
        // the module docs on `rules`. Like every other gate this runs on the
        // call, not only on the listing.
        if !self.inner.rules.permits(Call {
            target: &target.name,
            subject: Subject::Tool(&tool),
            claims: who.claims(),
        }) {
            return Err(error(
                ErrorCode::INVALID_REQUEST,
                format!("tool `{federated}` is not permitted on this route"),
            ));
        }

        // The target's own filters gate the call too, for the same reason.
        if !target.filter.permits(&tool) {
            return Err(error(
                ErrorCode::INVALID_REQUEST,
                format!("tool `{federated}` is not exposed by this gateway"),
            ));
        }

        let mut params = request.clone();
        params.name = tool.clone();
        params.meta = forward_meta(&params.meta);

        // Guardrails run last of the gates: a processor is consulted only
        // about calls that were otherwise going to happen, and it sees the
        // unmuxed name the upstream will actually receive.
        let backends = vec![target.name.clone()];
        let mut upstream_headers = self
            .transformed::<()>(HeaderOverride::default(), Annotations::default())
            .headers;
        if self.inner.guardrails.runs_request(TOOLS_CALL) {
            let encoded = encode(&params);
            let decision = self.block(self.inner.guardrails.check_request(
                CallContext {
                    method: TOOLS_CALL,
                    headers: &who.headers,
                    claims: who.claims(),
                    subject: Some(Subject::Tool(&tool)),
                    target: Some(&target.name),
                },
                &backends,
                Some(&encoded),
            ));

            match decision.outcome {
                Outcome::Pass => {}
                Outcome::Mutated(body) => match decode(&body) {
                    Some(rewritten) => params = rewritten,
                    // A processor that returns something unusable is a
                    // processor that failed, so it takes the same path as one
                    // that could not be reached rather than being ignored.
                    None => {
                        tracing::warn!("a guardrail rewrote tools/call into something unusable");
                        return Err(error(
                            ErrorCode::INTERNAL_ERROR,
                            "mcpGuardrails returned an unusable request",
                        ));
                    }
                },
                Outcome::Reject {
                    code,
                    message,
                    data,
                } => return Err(refusal(code, message, data)),
            }

            span::annotate(&span, &decision.annotations);
            upstream_headers = self
                .transformed::<()>(decision.headers.into(), decision.annotations)
                .headers;
        }

        // An upstream failure skips the response phase. There is no result to
        // inspect, and asking a guardrail to approve a failure is not a
        // question it can answer.
        let result = match target.call(&params, &upstream_headers) {
            Ok(result) => result,
            Err(ClientError::Timeout) => {
                tracing::warn!(
                    target = %target.name,
                    tool = %federated,
                    "tool call exceeded the backend budget"
                );
                // An error the caller can read, not a protocol error: the
                // request was well-formed and the model deserves to be told
                // the tool timed out rather than shown an opaque internal
                // failure.
                let waited = self
                    .inner
                    .backend_timeout
                    .map_or_else(String::new, |b| format!(" after {}ms", b.as_millis()));
                return Ok(CallToolResult {
                    content: vec![ContentBlock::text(format!(
                        "`{federated}` timed out{waited}"
                    ))],
                    is_error: Some(true),
                    ..CallToolResult::default()
                });
            }
            Err(err) => {
                tracing::warn!(target = %target.name, tool = %federated, %err, "tool call failed");
                return Err(error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("calling `{federated}` failed: {err}"),
                ));
            }
        };

        self.guard_response(
            TOOLS_CALL,
            &backends,
            &result,
            Some((Subject::Tool(&tool), &target.name)),
            &who,
        )
    }
}

impl PromptSource for Federation {
    fn prompts(&self, ctx: &Call_) -> Result<Vec<Prompt>, ErrorData> {
        let who = Who::of(ctx);
        let claims = who.claims();
        let targets: Vec<&Target> = self
            .inner
            .targets
            .iter()
            .filter(|t| t.serves_prompts())
            .collect();
        let backends: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();

        let span = span::request(PROMPTS_LIST, ctx);

        let guarded = self.guard_request(PROMPTS_LIST, &backends, None, None, &who)?;
        span::annotate(&span, &guarded.annotations);
        let headers = guarded.headers;

        let mut prompts: Vec<Prompt> = Vec::new();
        let mut index = HashMap::new();

        for target in targets {
            let Some(upstream) = self.upstream(target, "prompts", || target.prompts(&headers))
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

        *write(&self.inner.prompt_index) = index;

        let listing = ListPromptsResult {
            prompts,
            ..Default::default()
        };
        self.guard_response(PROMPTS_LIST, &backends, &listing, None, &who)
            .map(|l| l.prompts)
    }

    fn get(
        &self,
        ctx: &Call_,
        request: &GetPromptParams,
    ) -> Option<Result<GetPromptResult, ErrorData>> {
        Some(self.get_prompt(ctx, request))
    }
}

impl Federation {
    fn get_prompt(
        &self,
        ctx: &Call_,
        request: &GetPromptParams,
    ) -> Result<GetPromptResult, ErrorData> {
        let federated = request.name.clone();
        let span = span::request(PROMPTS_GET, ctx);
        let who = Who::of(ctx);

        let Some((target, name)) = self.route_prompt(&federated) else {
            return Err(error(
                ErrorCode::INVALID_PARAMS,
                format!("unknown prompt `{federated}`"),
            ));
        };

        // Checked on the fetch, not only on the listing. Nothing stops a
        // client asking for a name it was never shown.
        self.permit(&target.name, Subject::Prompt(&name), &federated, &who)?;

        let mut params = request.clone();
        params.name = name.clone();
        params.meta = forward_meta(&params.meta);

        let backends = vec![target.name.clone()];
        let guarded = self.guard_request_json(
            PROMPTS_GET,
            &backends,
            &params,
            Some((Subject::Prompt(&params.name), &target.name)),
            &who,
        )?;
        span::annotate(&span, &guarded.annotations);
        let (params, headers) = (guarded.body.unwrap_or(params), guarded.headers);
        // What was actually fetched, which a request-phase rewrite may have
        // changed. The response phase should describe the result it is looking
        // at, not the name the client happened to ask for.
        let fetched = params.name.clone();

        let result = self
            .upstream(target, "prompts/get", || {
                target.get_prompt(&params, &headers)
            })
            .ok_or_else(|| {
                error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("fetching `{federated}` failed"),
                )
            })?;

        self.guard_response(
            PROMPTS_GET,
            &backends,
            &result,
            Some((Subject::Prompt(&fetched), &target.name)),
            &who,
        )
    }
}

impl ResourceSource for Federation {
    fn resources(&self, ctx: &Call_) -> Result<Vec<Resource>, ErrorData> {
        let who = Who::of(ctx);
        let claims = who.claims();
        let targets: Vec<&Target> = self
            .inner
            .targets
            .iter()
            .filter(|t| t.serves_resources())
            .collect();
        let backends: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();

        let span = span::request(RESOURCES_LIST, ctx);

        let guarded = self.guard_request(RESOURCES_LIST, &backends, None, None, &who)?;
        span::annotate(&span, &guarded.annotations);
        let headers = guarded.headers;

        let mut resources: Vec<Resource> = Vec::new();
        let mut index = HashMap::new();

        for target in targets {
            let Some(upstream) =
                self.upstream(target, "resources", || target.resources(&headers))
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

        *write(&self.inner.resource_index) = index;

        let listing = ListResourcesResult {
            resources,
            ..Default::default()
        };
        self.guard_response(RESOURCES_LIST, &backends, &listing, None, &who)
            .map(|l| l.resources)
    }

    fn templates(&self, ctx: &Call_) -> Result<Vec<ResourceTemplate>, ErrorData> {
        let who = Who::of(ctx);
        let claims = who.claims();
        let targets: Vec<&Target> = self
            .inner
            .targets
            .iter()
            .filter(|t| t.serves_resources())
            .collect();
        let backends: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();

        let span = span::request(RESOURCES_TEMPLATES_LIST, ctx);

        let guarded = self.guard_request(RESOURCES_TEMPLATES_LIST, &backends, None, None, &who)?;
        span::annotate(&span, &guarded.annotations);
        let headers = guarded.headers;

        let mut templates: Vec<ResourceTemplate> = Vec::new();

        for target in targets {
            let Some(upstream) = self.upstream(target, "resources/templates", || {
                target.resource_templates(&headers)
            }) else {
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
        self.guard_response(RESOURCES_TEMPLATES_LIST, &backends, &listing, None, &who)
            .map(|l| l.resource_templates)
    }

    fn read(
        &self,
        ctx: &Call_,
        request: &ReadResourceParams,
    ) -> Option<Result<ReadResourceResult, ErrorData>> {
        Some(self.read_resource(ctx, request))
    }
}

impl Federation {
    fn read_resource(
        &self,
        ctx: &Call_,
        request: &ReadResourceParams,
    ) -> Result<ReadResourceResult, ErrorData> {
        let federated = request.uri.clone();
        let span = span::request(RESOURCES_READ, ctx);
        let who = Who::of(ctx);

        let Some((target, uri)) = self.route_resource(&federated) else {
            return Err(error(
                ErrorCode::INVALID_PARAMS,
                format!("unknown resource `{federated}`"),
            ));
        };

        self.permit(&target.name, Subject::Resource(&uri), &federated, &who)?;

        let mut params = request.clone();
        params.uri = uri.clone();
        params.meta = forward_meta(&params.meta);

        let backends = vec![target.name.clone()];
        let guarded = self.guard_request_json(
            RESOURCES_READ,
            &backends,
            &params,
            Some((Subject::Resource(&params.uri), &target.name)),
            &who,
        )?;
        span::annotate(&span, &guarded.annotations);
        let (mut params, headers) = (guarded.body.unwrap_or(params), guarded.headers);

        // The upstream knows its own URI, never the federated one. A guardrail
        // that rewrote the params could have put the federated form back.
        params.uri = strip_prefix(&self.inner.namer, &target.name, params.uri);
        // What was actually read, which a request-phase rewrite may have
        // changed.
        let read = params.uri.clone();

        let mut result = self
            .upstream(target, "resources/read", || {
                target.read_resource(&params, &headers)
            })
            .ok_or_else(|| {
                error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("reading `{federated}` failed"),
                )
            })?;

        // Contents come back carrying the target's own URIs, which no client
        // could read back to us. Re-qualify them so the round trip closes.
        for content in &mut result.contents {
            let (ResourceContents::Text { uri, .. } | ResourceContents::Blob { uri, .. }) =
                content;
            *uri = self.inner.namer.qualify_uri(&target.name, uri);
        }

        self.guard_response(
            RESOURCES_READ,
            &backends,
            &result,
            Some((Subject::Resource(&read), &target.name)),
            &who,
        )
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
    /// Run the async guardrail chain from a worker thread.
    fn block<F: std::future::Future>(&self, future: F) -> F::Output {
        self.inner.runtime.block_on(future)
    }

    /// Route a federated prompt name to its target and the target's own name.
    fn route_prompt(&self, federated: &str) -> Option<(&Target, String)> {
        match self.inner.namer.resolve(federated) {
            Resolution::Qualified { target, tool } => {
                let name = tool.to_string();
                self.target(target).map(|t| (t, name))
            }
            Resolution::Unqualified(name) => {
                let owner = read(&self.inner.prompt_index).get(name).cloned()?;
                self.target(&owner).map(|t| (t, name.to_string()))
            }
        }
    }

    /// Route a federated resource URI to its target and the target's own URI.
    fn route_resource(&self, federated: &str) -> Option<(&Target, String)> {
        match self.inner.namer.resolve_uri(federated) {
            Resolution::Qualified { target, tool } => {
                let uri = tool.to_string();
                self.target(target).map(|t| (t, uri))
            }
            Resolution::Unqualified(uri) => {
                let owner = read(&self.inner.resource_index).get(uri).cloned()?;
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
        who: &Who,
    ) -> Result<(), ErrorData> {
        if self.inner.rules.permits(Call {
            target,
            subject,
            claims: who.claims(),
        }) {
            return Ok(());
        }
        Err(error(
            ErrorCode::INVALID_REQUEST,
            format!(
                "{} `{federated}` is not permitted on this route",
                subject.noun()
            ),
        ))
    }

    /// An upstream call, logging (and swallowing) what gave out: one unhealthy
    /// target must not blank the whole listing. The call is bounded by the
    /// backend budget, which the target's connections enforce.
    fn upstream<T>(
        &self,
        target: &Target,
        what: &str,
        call: impl FnOnce() -> Result<T, ClientError>,
    ) -> Option<T> {
        match call() {
            Ok(value) => Some(value),
            Err(ClientError::Timeout) => {
                tracing::warn!(
                    target = %target.name,
                    what,
                    timeout_ms = self
                        .inner
                        .backend_timeout
                        .map_or(0, |b| b.as_millis() as u64),
                    "an upstream call exceeded the backend budget"
                );
                None
            }
            Err(err) => {
                tracing::warn!(target = %target.name, what, %err, "an upstream call failed");
                None
            }
        }
    }

    /// Run the request phase for a method that carries no params.
    fn guard_request(
        &self,
        method: &str,
        backends: &[String],
        params: Option<&[u8]>,
        about: Option<(Subject<'_>, &str)>,
        who: &Who,
    ) -> Result<Guarded<()>, ErrorData> {
        // Still runs when no processor is keyed on this method: a route may
        // set a static header without any guardrail at all.
        if !self.inner.guardrails.runs_request(method) {
            return Ok(self.transformed(HeaderOverride::default(), Annotations::default()));
        }

        let decision = self.block(self.inner.guardrails.check_request(
            CallContext {
                method,
                headers: &who.headers,
                claims: who.claims(),
                subject: about.map(|(subject, _)| subject),
                target: about.map(|(_, target)| target),
            },
            backends,
            params,
        ));

        match decision.outcome {
            Outcome::Reject {
                code,
                message,
                data,
            } => Err(refusal(code, message, data)),
            _ => Ok(self.transformed(decision.headers.into(), decision.annotations)),
        }
    }

    /// The same, for a method whose params a processor may rewrite.
    fn guard_request_json<T: Wire>(
        &self,
        method: &str,
        backends: &[String],
        params: &T,
        about: Option<(Subject<'_>, &str)>,
        who: &Who,
    ) -> Result<Guarded<T>, ErrorData> {
        // Still runs when no processor is keyed on this method: a route may
        // set a static header without any guardrail at all.
        if !self.inner.guardrails.runs_request(method) {
            return Ok(self.transformed(HeaderOverride::default(), Annotations::default()));
        }

        let encoded = encode(params);
        let decision = self.block(self.inner.guardrails.check_request(
            CallContext {
                method,
                headers: &who.headers,
                claims: who.claims(),
                subject: about.map(|(subject, _)| subject),
                target: about.map(|(_, target)| target),
            },
            backends,
            Some(&encoded),
        ));

        let body = match decision.outcome {
            Outcome::Pass => None,
            Outcome::Mutated(raw) => match decode(&raw) {
                Some(rewritten) => Some(rewritten),
                // A processor that returns something unusable is a processor
                // that failed, so it takes the same path as one that could not
                // be reached rather than being ignored.
                None => {
                    tracing::warn!(method, "a guardrail rewrote a request into something unusable");
                    return Err(error(
                        ErrorCode::INTERNAL_ERROR,
                        "mcpGuardrails returned an unusable request",
                    ));
                }
            },
            Outcome::Reject {
                code,
                message,
                data,
            } => return Err(refusal(code, message, data)),
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

    /// Run the response phase over a value, returning it possibly rewritten.
    fn guard_response<T: Wire + Clone>(
        &self,
        method: &str,
        backends: &[String],
        value: &T,
        about: Option<(Subject<'_>, &str)>,
        who: &Who,
    ) -> Result<T, ErrorData> {
        if !self.inner.guardrails.runs_response(method) {
            return Ok(value.clone());
        }

        let encoded = encode(value);
        match self.block(self.inner.guardrails.check_response(
            CallContext {
                method,
                headers: &who.headers,
                claims: who.claims(),
                subject: about.map(|(subject, _)| subject),
                target: about.map(|(_, target)| target),
            },
            backends,
            &encoded,
        )) {
            Outcome::Pass => Ok(value.clone()),
            Outcome::Mutated(body) => decode(&body).ok_or_else(|| {
                tracing::warn!(method, "a guardrail rewrote a result into something unusable");
                error(
                    ErrorCode::INTERNAL_ERROR,
                    "mcpGuardrails returned an unusable result",
                )
            }),
            Outcome::Reject {
                code,
                message,
                data,
            } => Err(refusal(code, message, data)),
        }
    }
}
