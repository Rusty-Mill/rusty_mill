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

use agentgateway_auth::{AuthRejection, JwtAuthenticator};
use agentgateway_config::{BackendTarget, Config};
use agentgateway_core::{CorsDecision, CorsMatcher, Router};
use agentgateway_mcp::Federation;
use agentgateway_proxy::HostProxy;
use axum::body::Body;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, Request, StatusCode, header};
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
}

/// Per-route serving state.
struct RouteState {
    cors: Option<CorsMatcher>,
    jwt: Option<JwtAuthenticator>,
    /// Budget for producing a response on this route.
    timeout: Option<Duration>,
    backend: BackendState,
}

enum BackendState {
    /// A federated MCP server, optionally instrumented.
    Mcp {
        service: McpService,
        metrics: Option<McpMetricsLayer>,
    },
    /// One or more `host` upstreams, proxied over HTTP.
    Host(HostProxy),
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
        let default_timeout = config
            .config
            .as_ref()
            .and_then(|c| c.limits.as_ref())
            .and_then(|l| l.request_timeout)
            .map(Duration::from);

        let mut routes = Vec::with_capacity(router.route_count());
        routes.resize_with(router.route_count(), || RouteState {
            cors: None,
            jwt: None,
            timeout: None,
            backend: BackendState::Unsupported("route has no backend".into()),
        });

        for route in router.routes() {
            let at = route
                .name
                .clone()
                .unwrap_or_else(|| format!("route #{}", route.id));

            let cors = route.policies.cors.as_ref().map(CorsMatcher::new);

            // Built here, not per request: a `file:` JWKS that is missing or
            // malformed should stop the gateway booting rather than turn every
            // request into a 503.
            let jwt = match route.policies.jwt_auth.as_ref() {
                Some(policy) => Some(JwtAuthenticator::new(policy, &at)?),
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
                            BackendState::Host(proxy)
                        }
                    }
                }
                Some(other) => BackendState::Unsupported(format!(
                    "backend kind `{}` is not served by this build",
                    kind_name(other)
                )),
                None => BackendState::Unsupported("route has no backend".into()),
            };

            routes[route.id] = RouteState {
                cors,
                jwt,
                timeout,
                backend,
            };
        }

        Ok(Gateway { router, routes })
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

        // Authentication runs after the preflight branch above and before the
        // backend. The ordering is load-bearing: browsers do not send
        // `Authorization` on a preflight, so requiring a token there would
        // make every cross-origin call fail before the real request is ever
        // sent.
        if let Some(jwt) = &state.jwt
            && let Err(rejection) = jwt.authenticate(request.headers()).await
        {
            return Ok(with_cors(reject(&rejection), cors_headers));
        }

        let call = self.dispatch(state, &selection, peer, request);

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
        request: Request<hyper::body::Incoming>,
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
            BackendState::Host(proxy) => proxy
                .proxy(request, selection.matched_prefix.as_deref(), peer)
                .await
                .map(Body::new),
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

fn status(code: StatusCode, message: &str) -> Response {
    (
        code,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message.to_string(),
    )
        .into_response()
}
