//! Transport wiring.
//!
//! [`serve`] takes any [`ServerHandler`] and a [`ServerConfig`] and runs it on
//! the configured transport, with graceful shutdown. Tool authors never touch
//! this module — they write a handler and call [`serve`].

use std::sync::Arc;

use axum::Router;
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
    config::{HttpConfig, ServerConfig, Transport},
    error::ServeError,
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
    match config.transport {
        Transport::Stdio => serve_stdio(factory).await,
        Transport::Http(http) => serve_http(factory, http).await,
    }
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
    let router = if http.legacy_sessions {
        let service = StreamableHttpService::new(
            {
                let factory = Arc::clone(&factory);
                move || factory()
            },
            Arc::new(LocalSessionManager::default()),
            transport_config,
        );
        mount(&http.path, service)
    } else {
        let service = StreamableHttpService::new(
            {
                let factory = Arc::clone(&factory);
                move || factory()
            },
            Arc::new(NeverSessionManager::default()),
            transport_config,
        );
        mount(&http.path, service)
    };

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
        "serving MCP over Streamable HTTP"
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(async move { token.cancelled_owned().await })
        .await?;

    tracing::info!("http transport closed");
    Ok(())
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
