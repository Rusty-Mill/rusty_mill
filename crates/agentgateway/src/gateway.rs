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

use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use agentgateway_a2a::{A2aGateway, Decision};
use agentgateway_auth::{AuthRejection, Authorization, ExtAuthz, JwtAuthenticator};
use agentgateway_config::{BackendTarget, Config};
use agentgateway_core::{CorsDecision, CorsMatcher, RateLimiter, Router};
use agentgateway_llm::LlmBackend;
use agentgateway_mcp::{Federation, TokenClaims};
use agentgateway_proxy::{Headers, HostProxy, RequestBody, Scheme};
use agentgateway_tls::{TlsBinds, TlsTerminator};
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
}

/// Per-route serving state.
struct RouteState {
    cors: Option<CorsMatcher>,
    rate_limit: Option<RateLimiter>,
    jwt: Option<JwtAuthenticator>,
    ext_authz: Option<ExtAuthz>,
    /// Budget for producing a response on this route.
    timeout: Option<Duration>,
    /// `responseHeaderModifier`, for backends that do not apply it themselves.
    ///
    /// The `host` proxy applies its own to the upstream's response. An MCP
    /// route has no upstream HTTP response to modify — `rmcp`'s transport
    /// consumes those — so the modifier acts on the response the gateway
    /// itself produces, which is the only one a client ever sees.
    mcp_response_headers: Option<Headers>,
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
    Ai(LlmBackend),
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
        // Certificates are read here, so a missing or malformed one stops the
        // gateway booting rather than failing every handshake later.
        let tls = TlsBinds::build(config)?;
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
            mcp_response_headers: None,
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
                        route.policies.request_header_modifier.as_ref(),
                        route.policies.url_rewrite.as_ref(),
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
                Some(BackendTarget::Host(_)) => {
                    // A route mixing `host` with a kind we cannot resolve would
                    // silently drop that share of its traffic onto the hosts,
                    // sending it somewhere the operator did not ask for.
                    match route
                        .backends
                        .iter()
                        .find(|b| !matches!(b.target, BackendTarget::Host(_)))
                    {
                        Some(other) => BackendState::Unsupported(format!(
                            "route mixes `host` with `{}`, which is not served by this build",
                            kind_name(&other.target)
                        )),
                        None => {
                            let proxy = HostProxy::new(&route.backends, &route.policies, &at)?;
                            tracing::info!(
                                route = %at,
                                endpoints = proxy.endpoint_count(),
                                "host backend ready"
                            );

                            let a2a = match route.policies.a2a.as_ref() {
                                Some(policy) => {
                                    let agents: Vec<String> = route
                                        .backends
                                        .iter()
                                        .filter_map(|b| match &b.target {
                                            BackendTarget::Host(host) => Some(host.clone()),
                                            _ => None,
                                        })
                                        .collect();
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
                    let backend = LlmBackend::new(ai, &route.policies, &at)?;
                    tracing::info!(
                        route = %at,
                        provider = backend.provider().name(),
                        "LLM backend ready"
                    );
                    BackendState::Ai(backend)
                }
                Some(other) => BackendState::Unsupported(format!(
                    "backend kind `{}` is not served by this build",
                    kind_name(other)
                )),
                None => BackendState::Unsupported("route has no backend".into()),
            };

            // Only for MCP: the `host` proxy compiles and applies its own,
            // and applying it twice would append `add` values twice.
            let mcp_response_headers =
                match (&backend, route.policies.response_header_modifier.as_ref()) {
                    (BackendState::Mcp { .. }, Some(modifier)) => Some(Headers::new(
                        modifier,
                        &format!("{at}.responseHeaderModifier"),
                    )?),
                    _ => None,
                };

            routes[route.id] = RouteState {
                cors,
                rate_limit,
                jwt,
                ext_authz,
                timeout,
                mcp_response_headers,
                backend,
            };
        }

        Ok(Gateway {
            router,
            routes,
            tls,
        })
    }

    /// The TLS terminator for `port`, if that bind terminates TLS.
    pub fn tls(&self, port: u16) -> Option<std::sync::Arc<TlsTerminator>> {
        self.tls.get(port)
    }

    /// Ports the gateway needs sockets on.
    pub fn ports(&self) -> Vec<u16> {
        self.router.ports().collect()
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
        if let Some(authz) = &state.ext_authz {
            let decision = authz
                .check(request.method(), request.uri().path(), request.headers())
                .await;
            match decision {
                Authorization::Allow(headers) => {
                    // Whatever the authorizer resolved -- a user id, a tenant
                    // -- travels on to the upstream.
                    for (name, value) in headers {
                        request.headers_mut().insert(name, value);
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

        let call = self.dispatch(state, &selection, peer, scheme, request);

        let response = match state.timeout {
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

        Ok(with_cors(response, cors_headers))
    }

    async fn dispatch(
        &self,
        state: &RouteState,
        selection: &agentgateway_core::Selection<'_>,
        peer: Option<IpAddr>,
        scheme: Scheme,
        request: Request<hyper::body::Incoming>,
    ) -> Response {
        match &state.backend {
            // `call` takes &mut self, but the service is cheap to clone and
            // clones share the session manager -- which is what makes an
            // Mcp-Session-Id issued on one request usable on the next.
            BackendState::Mcp { service, metrics } => {
                let mut response = match metrics {
                    // Layering per request is just a struct wrap; the
                    // instruments behind it are shared, which is what makes
                    // the counts add up.
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
                };

                // Applied to what the gateway is about to send the client.
                // CORS is added after this, so a modifier cannot strip the
                // headers that answer a preflight -- those are the gateway's
                // own protocol, not the route's payload.
                if let Some(headers) = &state.mcp_response_headers {
                    headers.apply(response.headers_mut());
                }
                response
            }
            BackendState::Host { proxy, a2a } => {
                let (parts, body) = request.into_parts();
                let prefix = selection.matched_prefix.as_deref();

                let Some(a2a) = a2a else {
                    let request = Request::from_parts(parts, RequestBody::Stream(body));
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
