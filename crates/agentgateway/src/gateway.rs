//! The data plane.
//!
//! A [`Gateway`] pairs a compiled [`Router`] with the per-route state that
//! serving actually needs — a live MCP federation, a compiled CORS policy, an
//! upstream authority — held in side tables indexed by
//! [`agentgateway_core::CompiledRoute::id`].
//!
//! Building those is async and can fail (an MCP target has to be dialled and
//! handshaked), which is why it happens once at startup rather than per
//! request. A request path that could spawn a subprocess is a request path
//! that can time out under load.

use std::convert::Infallible;
use std::sync::Arc;

use agentgateway_config::{BackendTarget, Config};
use agentgateway_core::{CorsDecision, CorsMatcher, Router};
use agentgateway_mcp::Federation;
use bytes::Bytes;
use http::{Request, Response, StatusCode, header};
use http_body_util::{BodyExt as _, Full, combinators::BoxBody};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tower_service::Service as _;

/// A federated MCP endpoint, ready to serve.
type McpService = StreamableHttpService<Federation, LocalSessionManager>;

/// The response body every backend produces.
///
/// This is the type `StreamableHttpService` hands back, and boxing everything
/// to match it keeps MCP's streaming responses streaming: collecting them into
/// a single buffer would defeat the point of an SSE transport.
pub type GatewayBody = BoxBody<Bytes, Infallible>;

/// The compiled data plane.
pub struct Gateway {
    router: Router,
    routes: Vec<RouteState>,
}

/// Per-route serving state.
struct RouteState {
    cors: Option<CorsMatcher>,
    backend: BackendState,
}

enum BackendState {
    /// A federated MCP server.
    Mcp(McpService),
    /// A backend we parsed but cannot serve. The reason is returned to the
    /// client rather than logged and forgotten, so a misconfiguration is
    /// visible from the outside instead of looking like a routing miss.
    Unsupported(String),
}

impl Gateway {
    /// Build the data plane, connecting every MCP backend.
    pub async fn build(config: &Config) -> anyhow::Result<Self> {
        let router = Router::build(config)?;
        let mut routes = Vec::with_capacity(router.route_count());
        routes.resize_with(router.route_count(), || RouteState {
            cors: None,
            backend: BackendState::Unsupported("route has no backend".into()),
        });

        for route in router.routes() {
            let at = route
                .name
                .clone()
                .unwrap_or_else(|| format!("route #{}", route.id));

            let cors = route.policies.cors.as_ref().map(CorsMatcher::new);

            let backend = match route.backends.first().map(|b| &b.target) {
                Some(BackendTarget::Mcp(mcp)) => {
                    let federation = Federation::connect(
                        mcp,
                        route.policies.mcp_authorization.as_ref(),
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

                    let federation = Arc::new(federation);
                    BackendState::Mcp(StreamableHttpService::new(
                        move || Ok(Federation::clone(&federation)),
                        Arc::new(LocalSessionManager::default()),
                        StreamableHttpServerConfig::default(),
                    ))
                }
                Some(other) => BackendState::Unsupported(format!(
                    "backend kind `{}` is not served by this build",
                    kind_name(other)
                )),
                None => BackendState::Unsupported("route has no backend".into()),
            };

            routes[route.id] = RouteState { cors, backend };
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
        request: Request<hyper::body::Incoming>,
    ) -> Result<Response<GatewayBody>, Infallible> {
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

        let mut response = match &state.backend {
            BackendState::Mcp(service) => {
                // `call` takes &mut self, but the service is cheap to clone and
                // clones share the session manager -- which is what makes an
                // Mcp-Session-Id issued on one request usable on the next.
                let mut service = service.clone();
                match service.call(request).await {
                    Ok(response) => response,
                    Err(never) => match never {},
                }
            }
            BackendState::Unsupported(reason) => status(StatusCode::NOT_IMPLEMENTED, reason),
        };

        if let Some(headers) = cors_headers {
            response.headers_mut().extend(headers);
        }
        Ok(response)
    }
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

fn status(code: StatusCode, message: &str) -> Response<GatewayBody> {
    fn body(message: &str) -> GatewayBody {
        Full::new(Bytes::from(message.to_string()))
            .map_err(|never| match never {})
            .boxed()
    }

    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body(message))
        .unwrap_or_else(|_| {
            let mut fallback = Response::new(body(""));
            *fallback.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            fallback
        })
}
