//! The data plane.
//!
//! A [`Gateway`] pairs a compiled [`Router`] with the per-route state that
//! serving actually needs — a live MCP federation, a compiled CORS policy, a
//! JWT authenticator, a timeout — held in side tables indexed by
//! [`agentgateway_core::CompiledRoute::id`].
//!
//! Building those is async and can fail (an MCP target has to be dialled and
//! handshaked, a JWKS file read), which is why it happens once at startup
//! rather than per request. A request path that could spawn a subprocess is a
//! request path that can time out under load.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use agentgateway_a2a::{A2aGateway, Decision};
use agentgateway_auth::{AuthRejection, Authorization, ExtAuthz, JwtAuthenticator};
use agentgateway_config::{BackendTarget, Config};
use agentgateway_core::{
    CorsDecision, CorsMatcher, RateLimiter, Registry, Router, resolve_backends,
};
use agentgateway_llm::LlmBackend;
use agentgateway_mcp::Override as McpOverride;
use agentgateway_mcp::{Federation, TokenClaims};
use agentgateway_proxy::{Headers, HostProxy, RequestBody, Rewrite, Scheme};
use agentgateway_tls::{Passthrough, TlsBinds, TlsTerminator};
use axum::body::Body;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt as _;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rusty_mcp::otel::metrics::{Instruments, McpMetricsLayer};
use tower_layer::Layer as _;
use tower_service::Service as _;

/// A federated MCP endpoint, ready to serve.
type McpService = StreamableHttpService<Federation, LocalSessionManager>;

/// The compiled data plane.
pub struct Gateway {
    router: Router,
    routes: Vec<RouteState>,
    tls: TlsBinds,
    /// Ports that forward connections rather than serving requests on them.
    passthrough: BTreeMap<u16, Arc<Passthrough>>,
}

/// Resolve a route's `urlRewrite` into the parts of a single MCP target's
/// address it replaces.
///
/// An MCP route terminates the protocol rather than forwarding a request line,
/// so the path a rewrite acts on is the target's own configured path rather
/// than anything derived from the request — the session is dialled once at
/// startup, long before a request exists. `prefix` therefore replaces the
/// route's matched path prefix at the head of *that* path, which is the same
/// operation the proxy performs, on the only path this model has.
///
/// Empty unless there is exactly one target: with more than one, "the address"
/// has no answer. `Config::lint` reports every case this returns nothing for.
fn mcp_overrides(
    policies: &agentgateway_config::Policies,
    matches: &[agentgateway_core::RouteMatcher],
    backend: &agentgateway_config::McpBackend,
    at: &str,
) -> anyhow::Result<Vec<McpOverride>> {
    // `via` points the whole federation at one address; `urlRewrite.authority`
    // redirects a single target to a new one. Both land in the same place, so
    // when a config asks for both the backend's own field wins -- it is the
    // more specific of the two, and it is the one that names targets.
    let via = match backend.via.as_deref() {
        Some(raw) => Some(authority(raw, &format!("{at}.backends.mcp.via"))?),
        None => None,
    };

    let Some(rewrite) = policies.url_rewrite.as_ref() else {
        return Ok(backend
            .targets
            .iter()
            .map(|_| McpOverride {
                path: None,
                authority: via.clone(),
            })
            .collect());
    };

    // A path rewrite generalises across targets: it transforms each target's
    // own configured path and leaves its host alone, so a federation of
    // servers that agree on a path layout can be moved together.
    //
    // An authority does not. Applied to several targets it would point them
    // all at the same server -- not a redirect but a collapse, since a
    // target's address is what distinguishes it from the others. `Config::lint`
    // reports that rather than doing it.
    let redirect = match rewrite
        .authority
        .as_deref()
        .filter(|_| backend.targets.len() == 1 && via.is_none())
    {
        Some(raw) => Some(authority(raw, &format!("{at}.urlRewrite.authority"))?),
        None => None,
    };
    let replacement = via.or(redirect);

    Ok(backend
        .targets
        .iter()
        .map(|target| {
            // Only an `mcp:` target has an address; a `stdio` one speaks over
            // a pipe, and a rewrite aimed at it is dropped rather than
            // quietly landing somewhere else.
            let agentgateway_config::McpTargetKind::Mcp(http) = &target.kind else {
                return McpOverride::default();
            };
            McpOverride {
                path: mcp_path(rewrite, matches, &http.path),
                authority: replacement.clone(),
            }
        })
        .collect())
}

/// Parse a replacement authority, refusing one that carries a credential.
///
/// An authority may legally hold `user:password@`, and that is the problem: a
/// credential in an upstream URI hides somewhere nobody thinks to look and is
/// sent on every request. `backendAuth` is where one belongs.
fn authority(raw: &str, at: &str) -> anyhow::Result<http::uri::Authority> {
    if raw.contains('@') {
        anyhow::bail!(
            "{at}: `{raw}` is not a valid authority: userinfo does not belong in an upstream \
             address, use `backendAuth`"
        );
    }
    http::uri::Authority::try_from(raw)
        .map_err(|_| anyhow::anyhow!("{at}: `{raw}` is not a valid authority"))
}

/// The replacement path, if the rewrite names one this route can resolve.
fn mcp_path(
    rewrite: &agentgateway_config::UrlRewrite,
    matches: &[agentgateway_core::RouteMatcher],
    configured: &str,
) -> Option<String> {
    use agentgateway_config::PathRewrite;

    // `Rewrite` rather than a second implementation of the same rules: a
    // `prefix` that trimmed slashes differently here than on the proxy path
    // would be a difference nobody could predict from the config.
    let compiled = Rewrite::new(rewrite, "").ok()?;

    let matched_prefix = match &rewrite.path {
        Some(PathRewrite::Prefix(_)) => Some(sole_path_prefix(matches)?),
        _ => None,
    };

    compiled.path(configured, matched_prefix)
}

/// The one `pathPrefix` this route matches on, if it matches on exactly one.
///
/// A `prefix` rewrite replaces whatever the request matched, and which prefix
/// that was is not knowable at startup — which is when a backend that
/// terminates its protocol resolves the address it dials. One prefix makes the
/// question answerable; zero or several do not, and the rewrite is dropped
/// rather than anchored on a guess.
fn sole_path_prefix(matches: &[agentgateway_core::RouteMatcher]) -> Option<&str> {
    let mut prefixes = matches
        .iter()
        .filter_map(agentgateway_core::RouteMatcher::path_prefix);
    let only = prefixes.next()?;
    prefixes.next().is_none().then_some(only)
}

/// Per-route serving state.
struct RouteState {
    cors: Option<CorsMatcher>,
    rate_limit: Option<RateLimiter>,
    jwt: Option<JwtAuthenticator>,
    ext_authz: Option<ExtAuthz>,
    /// Budget for producing a response on this route.
    timeout: Option<Duration>,
    /// `responseHeaderModifier`, applied to whatever the route's backend
    /// produced.
    ///
    /// One place for every backend kind. It used to live inside the `host`
    /// proxy, which meant it reached a proxied upstream response and nothing
    /// else: not an `ai` completion, not an A2A card or refusal the gateway
    /// answers itself, not an MCP response, since none of those go through the
    /// proxy. Applying it where the backends converge covers all of them and
    /// keeps one description true of every route.
    ///
    /// Still scoped to backend responses. A preflight, a JWT challenge and an
    /// `extAuthz` refusal are answered before dispatch and are the gateway's
    /// own, not the route's payload.
    response_headers: Option<Headers>,
    backend: BackendState,
}

enum BackendState {
    /// A federated MCP server, optionally instrumented.
    Mcp {
        service: McpService,
        metrics: Option<McpMetricsLayer>,
    },
    /// One or more `host` upstreams, proxied over HTTP.
    ///
    /// `a2a` is present when the route carries Agent2Agent traffic, which adds
    /// method gating and agent-card discovery in front of the same proxy.
    ///
    /// Both fields are boxed: an agent card and a weighted endpoint ring make
    /// this several times the size of every other variant, and this enum sits
    /// inline in each route's state, so the largest variant is what every
    /// route costs.
    Host {
        proxy: Box<HostProxy>,
        a2a: Option<Box<A2aGateway>>,
    },
    /// An LLM provider behind an OpenAI-compatible API.
    /// An LLM provider behind an OpenAI-compatible API.
    ///
    /// Boxed, like the `host` proxy above: this is much the largest variant,
    /// and every route pays its size otherwise -- including the ones that are
    /// a one-word `Unsupported`.
    Ai(Box<LlmBackend>),
    /// A backend we parsed but cannot serve. The reason is returned to the
    /// client rather than logged and forgotten, so a misconfiguration is
    /// visible from the outside instead of looking like a routing miss.
    Unsupported(String),
}

impl Gateway {
    /// Build the data plane, connecting every MCP backend.
    ///
    /// `instruments` is `None` when OpenTelemetry metrics are off, in which
    /// case no metrics layer is mounted at all rather than one recording into
    /// a void.
    pub async fn build(
        config: &Config,
        instruments: Option<Arc<Instruments>>,
    ) -> anyhow::Result<Self> {
        let router = Router::build(config)?;
        // The inventory a `service` backend resolves against, indexed once for
        // every route that names one.
        let registry = Registry::new(&config.services, &config.workloads, &config.backends);
        // Certificates are read here, so a missing or malformed one stops the
        // gateway booting rather than failing every handshake later.
        let tls = TlsBinds::build(config)?;

        // A passthrough port forwards rather than serves, so its routes are
        // compiled here rather than going through the router at all.
        let mut passthrough: BTreeMap<u16, Arc<Passthrough>> = BTreeMap::new();
        for (b, bind) in config.binds.iter().enumerate() {
            let routes: Vec<agentgateway_config::TcpRoute> = bind
                .listeners
                .iter()
                .filter(|listener| listener.passes_through())
                .flat_map(|listener| listener.tcp_routes.iter().cloned())
                .collect();
            if routes.is_empty() {
                continue;
            }
            let forwarder = Passthrough::new(&routes, &registry, &format!("binds[{b}]"))?;
            tracing::info!(port = bind.port, "passthrough listener ready");
            passthrough.insert(bind.port, Arc::new(forwarder));
        }
        let default_timeout = config
            .config
            .as_ref()
            .and_then(|c| c.limits.as_ref())
            .and_then(|l| l.request_timeout)
            .map(Duration::from);

        let mut routes = Vec::with_capacity(router.route_count());
        routes.resize_with(router.route_count(), || RouteState {
            cors: None,
            rate_limit: None,
            jwt: None,
            ext_authz: None,
            timeout: None,
            response_headers: None,
            backend: BackendState::Unsupported("route has no backend".into()),
        });

        for route in router.routes() {
            let at = route
                .name
                .clone()
                .unwrap_or_else(|| format!("route #{}", route.id));

            let cors = route.policies.cors.as_ref().map(CorsMatcher::new);
            let rate_limit = RateLimiter::new(&route.policies.local_rate_limit, &at)?;

            // Built here, not per request: a `file:` JWKS that is missing or
            // malformed should stop the gateway booting rather than turn every
            // request into a 503.
            let jwt = match route.policies.jwt_auth.as_ref() {
                Some(policy) => Some(JwtAuthenticator::new(policy, &at)?),
                None => None,
            };

            let ext_authz = match route.policies.ext_authz.as_ref() {
                Some(policy) => Some(ExtAuthz::new(policy, &at)?),
                None => None,
            };

            // A route's own budget wins over the process-wide default.
            let timeout = route
                .policies
                .timeout
                .as_ref()
                .and_then(|t| t.request_timeout)
                .map(Duration::from)
                .or(default_timeout);

            // The budget that actually bounds a tool call. `requestTimeout`
            // cannot: the Streamable HTTP transport sends its SSE response
            // headers immediately and streams the result afterwards, so a tool
            // only starts running once the response has been produced.
            let backend_timeout = route
                .policies
                .timeout
                .as_ref()
                .and_then(|t| t.backend_request_timeout)
                .map(Duration::from);

            let backend = match route.backends.first().map(|b| &b.target) {
                Some(BackendTarget::Mcp(mcp)) => {
                    let federation = Federation::connect(
                        mcp,
                        route.policies.mcp_authorization.as_ref(),
                        route.policies.mcp_guardrails.as_ref(),
                        &registry,
                        route.policies.request_header_modifier.as_ref(),
                        mcp_overrides(&route.policies, &route.matches, mcp, &at)?,
                        backend_timeout,
                        &at,
                    )
                    .await?;

                    for reason in federation.degraded() {
                        tracing::warn!(route = %at, "serving degraded: {reason}");
                    }
                    tracing::info!(
                        route = %at,
                        targets = ?federation.target_names().collect::<Vec<_>>(),
                        "MCP backend ready"
                    );

                    // Label cardinality is bounded by the tools this route
                    // actually federates. Without that list, every unknown name
                    // a client invents would mint a new time series -- which is
                    // how a metrics backend gets taken down from the outside.
                    let metrics = instruments.as_ref().map(|instruments| {
                        McpMetricsLayer::new(Arc::clone(instruments))
                            .with_known_names(federation.tool_names())
                    });

                    let federation = Arc::new(federation);
                    BackendState::Mcp {
                        service: StreamableHttpService::new(
                            move || Ok(Federation::clone(&federation)),
                            Arc::new(LocalSessionManager::default()),
                            StreamableHttpServerConfig::default(),
                        ),
                        metrics,
                    }
                }
                // `host` and `service` are one kind here: both forward bytes,
                // and a service is resolved to addresses before the proxy sees
                // it. A route may mix them freely.
                Some(BackendTarget::Host(_) | BackendTarget::Service(_)) => {
                    // A route mixing them with a kind we cannot resolve would
                    // silently drop that share of its traffic onto the rest,
                    // sending it somewhere the operator did not ask for.
                    match route.backends.iter().find(|b| {
                        !matches!(b.target, BackendTarget::Host(_) | BackendTarget::Service(_))
                    }) {
                        Some(other) => BackendState::Unsupported(format!(
                            "route mixes forwarding backends with `{}`, which is not served by \
                             this build",
                            kind_name(&other.target)
                        )),
                        None => {
                            let resolved = resolve_backends(&route.backends, &registry, &at)?;
                            let proxy = HostProxy::new(&resolved, &route.policies, &at)?;
                            tracing::info!(
                                route = %at,
                                endpoints = proxy.endpoint_count(),
                                "forwarding backend ready"
                            );

                            let a2a = match route.policies.a2a.as_ref() {
                                Some(policy) => {
                                    // Card discovery follows a rewritten
                                    // authority. Forwarded calls already go
                                    // there, and a gateway that fetched cards
                                    // from an address it never sends traffic to
                                    // would serve a card describing the wrong
                                    // agents -- or, behind an egress proxy that
                                    // is the only route to them, none at all.
                                    //
                                    // A path rewrite deliberately does not
                                    // follow: the well-known path is the A2A
                                    // spec's, not the route's, and asking for a
                                    // card somewhere else finds nothing.
                                    let redirect = route
                                        .policies
                                        .url_rewrite
                                        .as_ref()
                                        .and_then(|rewrite| rewrite.authority.as_deref());
                                    let mut agents: Vec<String> = route
                                        .backends
                                        .iter()
                                        .filter_map(|b| match &b.target {
                                            BackendTarget::Host(host) => {
                                                Some(redirect.unwrap_or(host.as_str()).to_string())
                                            }
                                            _ => None,
                                        })
                                        .collect();
                                    // Every backend behind one rewritten
                                    // authority is the same address -- the
                                    // proxy already sends them all there -- and
                                    // fetching that card once per backend would
                                    // merge an agent with itself.
                                    agents.dedup();
                                    Some(A2aGateway::build(policy, &agents, &at).await?)
                                }
                                None => None,
                            };

                            BackendState::Host {
                                proxy: Box::new(proxy),
                                a2a: a2a.map(Box::new),
                            }
                        }
                    }
                }
                Some(BackendTarget::Ai(ai)) => {
                    let backend = LlmBackend::new(
                        ai,
                        &route.policies,
                        sole_path_prefix(&route.matches),
                        &at,
                    )?;
                    tracing::info!(
                        route = %at,
                        provider = backend.provider().name(),
                        endpoint = backend.endpoint(),
                        "LLM backend ready"
                    );
                    BackendState::Ai(Box::new(backend))
                }
                Some(BackendTarget::Dynamic(backend)) => {
                    // Sole backend only: the others name a fixed address and
                    // this one names none, so a weight between them would be a
                    // share of "wherever the client asked for", which is not a
                    // destination anybody can reason about.
                    match route.backends.len() {
                        1 => {
                            let proxy = HostProxy::dynamic(backend, &route.policies, &at)?;
                            tracing::warn!(
                                route = %at,
                                computed = proxy.is_dynamic(),
                                "a `dynamic` backend makes this route a forward proxy: the \
                                 client chooses the upstream, so put authentication and \
                                 authorization in front of it or anyone who can reach this \
                                 listener can reach anything the gateway can"
                            );
                            BackendState::Host {
                                proxy: Box::new(proxy),
                                a2a: None,
                            }
                        }
                        count => BackendState::Unsupported(format!(
                            "a `dynamic` backend takes its upstream from the request, so it \
                             cannot be weighted against the {} other backend(s) on this route",
                            count - 1
                        )),
                    }
                }
                // Every kind the configuration can name is served now, so
                // there is no catch-all left: a new one would fail to compile
                // here rather than reach a client as a 501.
                None => BackendState::Unsupported("route has no backend".into()),
            };

            let response_headers = match route.policies.response_header_modifier.as_ref() {
                Some(modifier) => Some(Headers::new(
                    modifier,
                    &format!("{at}.responseHeaderModifier"),
                )?),
                None => None,
            };

            routes[route.id] = RouteState {
                cors,
                rate_limit,
                jwt,
                ext_authz,
                timeout,
                response_headers,
                backend,
            };
        }

        Ok(Gateway {
            router,
            routes,
            tls,
            passthrough,
        })
    }

    /// The TLS terminator for `port`, if that bind terminates TLS.
    pub fn tls(&self, port: u16) -> Option<std::sync::Arc<TlsTerminator>> {
        self.tls.get(port)
    }

    /// The forwarder for `port`, if that bind passes connections through.
    ///
    /// A port is one or the other. Terminating some connections on a port and
    /// forwarding others would mean deciding per connection whether to present
    /// a certificate, and nothing in the configuration says which.
    pub fn passthrough(&self, port: u16) -> Option<Arc<Passthrough>> {
        self.passthrough.get(&port).cloned()
    }

    /// Ports the gateway needs sockets on.
    pub fn ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self.router.ports().collect();
        for port in self.passthrough.keys() {
            if !ports.contains(port) {
                ports.push(*port);
            }
        }
        ports
    }

    /// Serve one request that arrived on `port`.
    pub async fn handle(
        &self,
        port: u16,
        peer: Option<IpAddr>,
        scheme: Scheme,
        request: Request<hyper::body::Incoming>,
    ) -> Result<Response, Infallible> {
        let Some(selection) = self.router.select(port, &request) else {
            return Ok(status(
                StatusCode::NOT_FOUND,
                "no route matched this request",
            ));
        };

        let state = &self.routes[selection.route.id];

        // CORS is evaluated before the backend so a preflight is answered here
        // rather than forwarded upstream -- an MCP server has no reason to
        // know what a browser preflight is.
        let cors_headers = match state.cors.as_ref().map(|c| c.evaluate(&request)) {
            Some(CorsDecision::Preflight(headers)) => {
                let mut response = status(StatusCode::NO_CONTENT, "");
                response.headers_mut().extend(headers);
                return Ok(response);
            }
            Some(CorsDecision::Simple(headers)) => Some(headers),
            Some(CorsDecision::NotCors) | None => None,
        };

        // Rate limiting runs before authentication, so a flood is refused
        // before it costs a signature verification and possibly a JWKS fetch.
        // It runs after the preflight branch for the same reason auth does: a
        // browser reports a refused preflight as an opaque CORS error, so
        // rate limiting one hides the 429 the caller needs to see.
        if let Some(limiter) = &state.rate_limit
            && let Err(retry_after) = limiter.check()
        {
            let mut response = status(
                StatusCode::TOO_MANY_REQUESTS,
                "the rate limit for this route has been exceeded",
            );
            if let Ok(value) = http::HeaderValue::try_from(retry_after.seconds().to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            return Ok(with_cors(response, cors_headers));
        }

        // Authentication runs after the preflight branch above and before the
        // backend. The ordering is load-bearing: browsers do not send
        // `Authorization` on a preflight, so requiring a token there would
        // make every cross-origin call fail before the real request is ever
        // sent.
        let mut request = request;
        if let Some(jwt) = &state.jwt {
            match jwt.authenticate(request.headers()).await {
                // The claims travel on the request so `mcpAuthorization.rules`
                // can read `jwt.*`. Only a token this gateway verified is ever
                // put here, so a rule cannot be fooled by a header a caller
                // set itself.
                Ok(token) => {
                    request.extensions_mut().insert(TokenClaims(token.claims));
                }
                Err(rejection) => return Ok(with_cors(reject(&rejection), cors_headers)),
            }
        }

        // External authorization runs last of the gates, so it is asked only
        // about requests that got past the cheap local ones -- and so an
        // authorizer can see the identity `jwtAuth` just verified.
        let (parts, body) = request.into_parts();
        let mut parts = parts;
        let mut body = RequestBody::Stream(body);

        if let Some(authz) = &state.ext_authz {
            // Read the body only when the route asked for it. `includeBody` is
            // a bound and a body over it is refused rather than truncated: a
            // fragment of JSON does not parse, so the authorizer would answer
            // about something that was never the request.
            let buffered = match authz.include_body() {
                Some(limit) => match collect_limited(body, limit).await {
                    Ok(bytes) => {
                        body = RequestBody::Buffered(bytes.clone());
                        Some(bytes)
                    }
                    Err(()) => {
                        tracing::info!(
                            route = ?selection.route.name,
                            limit,
                            "refusing a request whose body exceeds `extAuthz.includeBody`"
                        );
                        return Ok(with_cors(
                            status(
                                StatusCode::PAYLOAD_TOO_LARGE,
                                "the request body is larger than `extAuthz.includeBody`, and \
                                 the authorization service is not asked to decide on part of one",
                            ),
                            cors_headers,
                        ));
                    }
                },
                None => None,
            };

            let decision = authz
                .check(
                    &parts.method,
                    parts.uri.path(),
                    &parts.headers,
                    buffered.as_deref(),
                )
                .await;
            match decision {
                Authorization::Allow(headers) => {
                    // Whatever the authorizer resolved -- a user id, a tenant
                    // -- travels on to the upstream.
                    for (name, value) in headers {
                        parts.headers.insert(name, value);
                    }
                }
                Authorization::Deny {
                    status,
                    headers,
                    body,
                } => {
                    tracing::info!(route = ?selection.route.name, %status, "external authorization denied");
                    let mut response = status_bytes(status, Bytes::from(body));
                    response.headers_mut().extend(headers);
                    return Ok(with_cors(response, cors_headers));
                }
            }
        }

        let request = Request::from_parts(parts, body);
        let call = self.dispatch(state, &selection, peer, scheme, request);

        let mut response = match state.timeout {
            Some(budget) => match tokio::time::timeout(budget, call).await {
                Ok(response) => response,
                Err(_) => {
                    tracing::warn!(
                        route = ?selection.route.name,
                        timeout_ms = budget.as_millis() as u64,
                        "request exceeded its budget"
                    );
                    status(
                        StatusCode::GATEWAY_TIMEOUT,
                        "the request exceeded its timeout budget",
                    )
                }
            },
            None => call.await,
        };

        if let Some(headers) = &state.response_headers {
            headers.apply(response.headers_mut());
        }
        Ok(with_cors(response, cors_headers))
    }

    async fn dispatch(
        &self,
        state: &RouteState,
        selection: &agentgateway_core::Selection<'_>,
        peer: Option<IpAddr>,
        scheme: Scheme,
        request: Request<RequestBody>,
    ) -> Response {
        match &state.backend {
            // `call` takes &mut self, but the service is cheap to clone and
            // clones share the session manager -- which is what makes an
            // Mcp-Session-Id issued on one request usable on the next.
            BackendState::Mcp { service, metrics } => match metrics {
                // Layering per request is just a struct wrap; the instruments
                // behind it are shared, which is what makes the counts add up.
                Some(metrics) => {
                    let mut service = metrics.layer(service.clone());
                    match service.call(request).await {
                        Ok(response) => response,
                        Err(never) => match never {},
                    }
                }
                None => {
                    let mut service = service.clone();
                    match service.call(request).await {
                        Ok(response) => response.into_response(),
                        Err(never) => match never {},
                    }
                }
            },
            BackendState::Host { proxy, a2a } => {
                let (parts, body) = request.into_parts();
                let prefix = selection.matched_prefix.as_deref();

                let Some(a2a) = a2a else {
                    let request = Request::from_parts(parts, body);
                    return proxy
                        .proxy(request, prefix, peer, scheme)
                        .await
                        .map(Body::new);
                };

                // Discovery is answered here rather than forwarded: the point
                // of the merged card is that it names the gateway, and an
                // agent's own card names the agent.
                if a2a.is_card_request(&parts.method, parts.uri.path()) {
                    return match a2a.card() {
                        Some(card) => json(StatusCode::OK, Bytes::copy_from_slice(card)),
                        None => status(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "no agent card could be assembled from the agents behind this route",
                        ),
                    };
                }

                // Gating needs the method, and the method is in the body, so
                // the body has to be read. It is handed to the proxy already
                // buffered rather than re-read.
                let Ok(collected) = body.collect().await else {
                    return status(StatusCode::BAD_REQUEST, "could not read the request body");
                };
                let bytes = collected.to_bytes();

                match a2a.check(&bytes) {
                    Decision::Refused { method, body } => {
                        tracing::info!(method = %method, "refusing an A2A method");
                        // 200 with a JSON-RPC error object, not an HTTP error:
                        // that is where a JSON-RPC client looks, and an A2A
                        // client parsing the envelope would otherwise see a
                        // transport failure instead of the reason.
                        return json(StatusCode::OK, Bytes::from(body));
                    }
                    Decision::Permitted { method } => {
                        tracing::debug!(
                            method = %method,
                            task = ?agentgateway_a2a::task_id(&bytes),
                            "forwarding an A2A call"
                        );
                    }
                    Decision::NotJsonRpc => {}
                }

                let request = Request::from_parts(parts, RequestBody::Buffered(bytes));
                proxy
                    .proxy(request, prefix, peer, scheme)
                    .await
                    .map(Body::new)
            }
            BackendState::Ai(backend) => backend.handle(request).await.map(Body::new),
            BackendState::Unsupported(reason) => status(StatusCode::NOT_IMPLEMENTED, reason),
        }
    }
}

/// Read a body, refusing rather than truncating once it passes `limit`.
///
/// `Err` means it was too large. Nothing partially read is handed back: the
/// caller either gets the whole body, which it can forward, or a refusal.
async fn collect_limited(body: RequestBody, limit: usize) -> Result<Bytes, ()> {
    http_body_util::Limited::new(body, limit)
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .map_err(|_| ())
}

fn with_cors(mut response: Response, headers: Option<HeaderMap>) -> Response {
    if let Some(headers) = headers {
        response.headers_mut().extend(headers);
    }
    response
}

/// Turn an authentication failure into a response.
fn reject(rejection: &AuthRejection) -> Response {
    let mut response = status(rejection.status, &rejection.description);
    if let Some(challenge) = rejection.challenge()
        && let Ok(value) = http::HeaderValue::try_from(challenge)
    {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    } else if rejection.status == StatusCode::UNAUTHORIZED {
        // A 401 without a challenge is a protocol violation, and a client that
        // gets one has no way to learn it should authenticate.
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            http::HeaderValue::from_static(agentgateway_auth::BEARER_CHALLENGE),
        );
    }
    response
}

fn kind_name(target: &BackendTarget) -> &'static str {
    match target {
        BackendTarget::Host(_) => "host",
        BackendTarget::Service(_) => "service",
        BackendTarget::Mcp(_) => "mcp",
        BackendTarget::Ai(_) => "ai",
        BackendTarget::Dynamic(_) => "dynamic",
    }
}

/// A response carrying the authorizer's own body, whatever it was.
fn status_bytes(code: StatusCode, body: Bytes) -> Response {
    (code, body).into_response()
}

fn json(code: StatusCode, body: Bytes) -> Response {
    (code, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn status(code: StatusCode, message: &str) -> Response {
    (
        code,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message.to_string(),
    )
        .into_response()
}
