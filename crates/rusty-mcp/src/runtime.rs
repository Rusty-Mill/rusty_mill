//! Transport wiring.
//!
//! [`serve`] takes any [`ServerHandler`] and a [`ServerConfig`] and runs it on
//! the configured transport, with graceful shutdown. Tool authors never touch
//! this module — they write a handler and call [`serve`].

use std::sync::Arc;

use axum::{Router, routing::get};
use rmcp::{
    ServerHandler, ServiceExt,
    transport::{
        io::stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
            session::{local::LocalSessionManager, never::NeverSessionManager},
        },
    },
};
use tokio_util::sync::CancellationToken;

use crate::{
    auth::{AuthConfig, ProtectedResourceMetadata, RequireAuthLayer},
    config::{HttpConfig, ServerConfig, Transport},
    error::ServeError,
    limits::LimitsLayer,
    shutdown,
};

/// Builds a fresh handler per connection.
///
/// Streamable HTTP calls this for each incoming request, so the handler must be
/// cheap to construct; put anything expensive behind an [`Arc`] captured by the
/// closure.
pub trait HandlerFactory<S>: Fn() -> Result<S, std::io::Error> + Send + Sync + 'static {}

impl<S, F> HandlerFactory<S> for F where F: Fn() -> Result<S, std::io::Error> + Send + Sync + 'static
{}

/// Run `factory`'s handler on the transport named by `config`.
///
/// Returns once the transport closes or a shutdown signal arrives. Logging is
/// the caller's job — call [`crate::telemetry::init`] first, or use
/// [`crate::run`], which does both.
pub async fn serve<S, F>(factory: F, config: ServerConfig) -> Result<(), ServeError>
where
    S: ServerHandler + Send + 'static,
    F: HandlerFactory<S>,
{
    let shutdown_hook = config.shutdown_hook.clone();

    let result = match config.transport {
        Transport::Stdio => serve_stdio(factory).await,
        Transport::Http(http) => serve_http(factory, http).await,
    };

    // Runs even when the transport failed: work spawned before the failure
    // still deserves a chance to finish, and dropping it silently is how
    // half-written state happens.
    if let Some(hook) = shutdown_hook {
        tracing::debug!("running shutdown hook");
        hook().await;
        tracing::debug!("shutdown hook complete");
    }

    result
}

async fn serve_stdio<S, F>(factory: F) -> Result<(), ServeError>
where
    S: ServerHandler + Send + 'static,
    F: HandlerFactory<S>,
{
    let handler = factory().map_err(ServeError::Handler)?;
    let token = shutdown::token();

    tracing::info!("serving MCP over stdio");

    let service = handler
        .serve_with_ct(stdio(), token.clone())
        .await
        .map_err(|err| ServeError::Transport(Box::new(err)))?;

    let reason = service
        .waiting()
        .await
        .map_err(|err| ServeError::Transport(Box::new(err)))?;

    tracing::info!(?reason, "stdio transport closed");
    Ok(())
}

async fn serve_http<S, F>(factory: F, http: HttpConfig) -> Result<(), ServeError>
where
    S: ServerHandler + Send + 'static,
    F: HandlerFactory<S>,
{
    let token = shutdown::token();
    let transport_config = build_transport_config(&http, token.clone());

    // The two session managers are distinct types, so each branch builds its
    // own router. `NeverSessionManager` is the honest default under SEP-2567:
    // it refuses to mint sessions at all.
    let factory = Arc::new(factory);
    let auth = http.auth.clone();

    #[cfg(feature = "otel")]
    let metrics = http.metrics.clone();
    #[cfg(not(feature = "otel"))]
    let metrics: MetricsLayer = None;
    let limits = http.limits.clone();

    let mut router = if http.legacy_sessions {
        let service = StreamableHttpService::new(
            {
                let factory = Arc::clone(&factory);
                move || factory()
            },
            Arc::new(LocalSessionManager::default()),
            transport_config,
        );
        mount_guarded(
            &http.path,
            service,
            auth.as_ref(),
            metrics.as_ref(),
            limits.as_ref(),
        )
    } else {
        let service = StreamableHttpService::new(
            {
                let factory = Arc::clone(&factory);
                move || factory()
            },
            Arc::new(NeverSessionManager::default()),
            transport_config,
        );
        mount_guarded(
            &http.path,
            service,
            auth.as_ref(),
            metrics.as_ref(),
            limits.as_ref(),
        )
    };

    // The metadata document must stay reachable *without* a token: it is how a
    // client that just received a 401 finds out where to authenticate. Adding
    // it outside the guarded route is what keeps that from deadlocking.
    if let Some(auth) = &auth {
        router = router.route(
            &auth.metadata_path(),
            get({
                let auth = Arc::clone(auth);
                move || {
                    let body = axum::Json(ProtectedResourceMetadata::from_config(&auth));
                    async move { body }
                }
            }),
        );
    }

    let listener = tokio::net::TcpListener::bind(http.bind)
        .await
        .map_err(|source| ServeError::Bind {
            bind: http.bind.to_string(),
            source,
        })?;

    let local_addr = listener.local_addr()?;
    tracing::info!(
        address = %local_addr,
        path = %http.path,
        legacy_sessions = http.legacy_sessions,
        authorized = http.auth.is_some(),
        "serving MCP over Streamable HTTP"
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(async move { token.cancelled_owned().await })
        .await?;

    tracing::info!("http transport closed");
    Ok(())
}

/// The metrics layer, or a type nothing can construct when `otel` is off.
///
/// `Infallible` rather than `()` on purpose: without the feature the `Some`
/// branch is uninhabited, so the compiler proves the metrics path is gone
/// rather than leaving a runtime `None` check behind.
#[cfg(feature = "otel")]
type MetricsLayer = Option<crate::otel::metrics::McpMetricsLayer>;
#[cfg(not(feature = "otel"))]
type MetricsLayer = Option<std::convert::Infallible>;

/// Mount the MCP service at `path`, wrapped in the layers that are configured.
///
/// Order is outside-in: **limits, metrics, authorization, handler** — cheapest
/// rejection first.
///
/// - Limits outermost, so a shed request costs a semaphore try-acquire and
///   nothing else. Inside the guard it would pay for a signature check before
///   being told there was no capacity for it anyway.
/// - Metrics outside authorization, so a request rejected with a `401` is still
///   counted. Inside, a flood of bad tokens would look like no traffic at all.
fn mount_guarded<T>(
    path: &str,
    service: T,
    auth: Option<&Arc<AuthConfig>>,
    metrics: Option<&<MetricsLayer as IntoInner>::Inner>,
    limits: Option<&LimitsLayer>,
) -> Router
where
    T: tower_service::Service<axum::extract::Request, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    T::Response: axum::response::IntoResponse,
    T::Future: Send + 'static,
{
    use tower_layer::Layer as _;

    match auth {
        Some(auth) => {
            tracing::info!(
                resource = auth.resource(),
                metadata = %auth.metadata_path(),
                required_scopes = ?auth.required_scopes,
                "requiring OAuth 2.1 bearer authorization"
            );
            mount_metered(
                path,
                RequireAuthLayer::from_shared(Arc::clone(auth)).layer(service),
                metrics,
                limits,
            )
        }
        None => mount_metered(path, service, metrics, limits),
    }
}

/// Names the `T` inside an `Option<T>`, so one signature covers both features.
trait IntoInner {
    type Inner;
}

impl<T> IntoInner for Option<T> {
    type Inner = T;
}

/// Mount `service`, wrapped in the metrics layer when one is configured.
fn mount_metered<T>(
    path: &str,
    service: T,
    metrics: Option<&<MetricsLayer as IntoInner>::Inner>,
    limits: Option<&LimitsLayer>,
) -> Router
where
    T: tower_service::Service<axum::extract::Request, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    T::Response: axum::response::IntoResponse,
    T::Future: Send + 'static,
{
    match metrics {
        #[cfg(feature = "otel")]
        Some(layer) => {
            use tower_layer::Layer as _;
            tracing::info!("recording request metrics");
            mount_limited(path, layer.layer(service), limits)
        }
        // Without the feature this arm is uninhabited, so the match is
        // exhaustive on `None` alone.
        #[cfg(not(feature = "otel"))]
        Some(never) => match *never {},
        None => mount_limited(path, service, limits),
    }
}

/// Mount `service`, wrapped in the limits layer when one is configured.
///
/// Outermost of the three. A shed request costs a semaphore try-acquire and
/// nothing else — no token validation, no JWKS lookup, no handler. Putting this
/// inside the auth layer would mean paying for a signature check on every
/// request in a flood before deciding there was no capacity for it anyway.
///
/// The trade is that shed requests are invisible to the metrics layer, since
/// they never reach it. That is the right way round: the `503` rate is what a
/// load balancer already sees, whereas a rejected token is only visible here.
fn mount_limited<T>(path: &str, service: T, limits: Option<&LimitsLayer>) -> Router
where
    T: tower_service::Service<axum::extract::Request, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    T::Response: axum::response::IntoResponse,
    T::Future: Send + 'static,
{
    use tower_layer::Layer as _;

    match limits {
        Some(layer) => {
            tracing::info!(
                max_concurrent = ?layer.max_concurrent(),
                timeout = ?layer.timeout(),
                "limiting inbound requests"
            );
            mount(path, layer.layer(service))
        }
        None => mount(path, service),
    }
}

/// Mount the MCP service at `path`.
///
/// `nest_service` rejects a bare `/`, so that case falls back to a catch-all.
fn mount<T>(path: &str, service: T) -> Router
where
    T: tower_service::Service<axum::extract::Request, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    T::Response: axum::response::IntoResponse,
    T::Future: Send + 'static,
{
    let normalized = normalize_path(path);
    if normalized == "/" {
        Router::new().fallback_service(service)
    } else {
        Router::new().nest_service(&normalized, service)
    }
}

/// Ensure a leading slash and no trailing slash (except for the root itself).
fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let with_leading = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    let stripped = with_leading.trim_end_matches('/');
    if stripped.is_empty() {
        "/".to_string()
    } else {
        stripped.to_string()
    }
}

fn build_transport_config(
    http: &HttpConfig,
    token: CancellationToken,
) -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default();
    config.sse_keep_alive = http.sse_keep_alive;
    config.legacy_session_mode = http.legacy_sessions;
    config.json_response = http.json_response;
    config.max_request_body_bytes = http.max_request_body_bytes;
    config.cancellation_token = token;

    if let Some(hosts) = &http.allowed_hosts {
        config = config.with_allowed_hosts(hosts.clone());
    }
    if let Some(origins) = &http.allowed_origins {
        config = config.with_allowed_origins(origins.clone());
    }
    config
}

#[cfg(test)]
mod tests {
    use super::normalize_path;

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_path("/mcp"), "/mcp");
        assert_eq!(normalize_path("mcp"), "/mcp");
        assert_eq!(normalize_path("/mcp/"), "/mcp");
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path("  /api/mcp  "), "/api/mcp");
    }
}
