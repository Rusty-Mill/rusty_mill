pub mod credentials;
pub mod errors;
pub mod jwt;
pub mod routes;
pub mod state;

use std::io;
use std::sync::Arc;
use std::time::Instant;

use rusty_axum::cors::{Allow, Cors};
use rusty_axum::middleware::{from_fn, Next};
use rusty_axum::routing::{any, get, patch, post};
use rusty_axum::{Request, Response, Router, ServeOptions, Server};
use rusty_axum_tokio::TokioBridge;
use rusty_mcp_server::{HttpConfig, HttpHandler, Mount};
use rusty_tokio::io::TcpListener;

use crate::state::AppState;

/// Builds the CORS layer from `state.cors_allowed_origins` -- `None`
/// (unset) keeps the original any-origin behavior; `Some` restricts to
/// exactly those origins. Methods/headers stay wildcard either way, since
/// this is scoped to origin restriction, not the rest of the CORS surface
/// -- and there's no credentialed (cookie-based) auth here for a wildcard
/// origin to be unsafe with. An entry that isn't a valid `Origin` header
/// value is skipped with a startup warning rather than failing the whole
/// list, same soft-failure posture as an invalid `[[guardrails]]` pattern.
fn build_cors(cors_allowed_origins: &Option<Vec<String>>) -> Cors {
    let Some(origins) = cors_allowed_origins else {
        return Cors::permissive();
    };
    let valid: Vec<String> = origins
        .iter()
        .filter(|origin| {
            let ok = is_header_value(origin);
            if !ok {
                rusty_log::warn!(origin = %origin, "skipping invalid cors_allowed_origins entry");
            }
            ok
        })
        .cloned()
        .collect();
    Cors::new()
        .allow_origin(Allow::List(valid))
        .allow_methods(Allow::Any)
        .allow_headers(Allow::Any)
}

/// What `http::HeaderValue::from_str` accepted: visible ASCII, space and tab.
fn is_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b == b'\t' || (b' '..=b'~').contains(&b))
}

/// Logs each request's method, path, status and latency at debug level,
/// in place of tower-http's `TraceLayer`.
async fn trace(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_owned();
    let path = request.path().to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    rusty_log::debug!(
        %method,
        %path,
        status = response.status.as_u16(),
        latency_ms = started.elapsed().as_millis() as u64,
        "request"
    );
    response
}

/// Routes the MCP endpoint at `state.mcp_path`, guarded by
/// [`routes::mcp`]'s `check_auth` (see that function's doc comment for why
/// this isn't an OAuth 2.1 resource-server auth of the MCP library's own).
///
/// The handler serves both generations of the protocol: clients that open
/// with the legacy `initialize` handshake (most desktop clients today) get a
/// session, and clients on spec 2026-07-28 use the stateless `server/discover`
/// bootstrap. Its default `Host`/`Origin` rules apply (loopback hosts, no
/// browser origins). An empty path (`/`) takes every path no other route
/// matches, as axum's `fallback_service` did.
fn mount_mcp(router: Router<AppState>, state: &AppState) -> Router<AppState> {
    let Some(mcp) = state.mcp.clone() else {
        return router;
    };

    let path = state.mcp_path.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let handler = Arc::new(HttpHandler::new(
        mcp,
        HttpConfig {
            path: path.to_owned(),
            ..HttpConfig::default()
        },
    ));
    let mount = Mount::new(handler);
    let guarded = move |rusty_axum::State(state): rusty_axum::State<AppState>, request: Request| {
        routes::mcp(state, mount.clone(), request)
    };

    if path == "/" {
        router.fallback(guarded)
    } else {
        router.route(path, any(guarded))
    }
}

/// Builds the full app (routes + middleware) over the given state. `tokio`
/// is the runtime the handlers' tokio libraries (reqwest's JWKS fetch, `tokio::sync`)
/// run on: the router's outermost layer enters it on every poll.
pub fn build_app(state: AppState, tokio: tokio::runtime::Handle) -> Router {
    let cors = build_cors(&state.cors_allowed_origins);
    let router = Router::new()
        .route("/health", get(routes::health))
        .route("/ready", get(routes::ready))
        .route("/dashboard", get(routes::dashboard))
        .route("/v1/models", get(routes::list_models))
        .route("/v1/usage", get(routes::usage_stats))
        .route("/v1/free-tiers", get(routes::free_tiers))
        .route("/v1/providers/stats", get(routes::provider_stats))
        .route("/v1/generation", get(routes::generation))
        .route("/metrics", get(routes::metrics))
        .route("/v1/chat/completions", post(routes::chat_completions))
        .route("/v1/embeddings", post(routes::embeddings))
        .route(
            "/v1/admin/clients",
            get(routes::admin_list_clients).post(routes::admin_create_client),
        )
        .route(
            "/v1/admin/organizations",
            get(routes::admin_list_organizations),
        )
        .route(
            "/v1/admin/clients/{name}",
            patch(routes::admin_update_client).delete(routes::admin_delete_client),
        )
        .route(
            "/v1/admin/clients/{name}/reset-spend",
            post(routes::admin_reset_client_spend),
        )
        .route(
            "/v1/admin/clients/{name}/usage-history",
            get(routes::admin_client_usage_history),
        );
    let limiter_state = state.clone();
    mount_mcp(router, &state)
        .with_state(state)
        .layer(from_fn(trace))
        .layer(cors)
        // A shed request costs a semaphore try-acquire and nothing else --
        // ahead of CORS and tracing, "cheapest rejection first". The body
        // limit is the server's (`serve`), applied before any layer.
        .layer(from_fn(move |request, next| {
            routes::concurrency_limit(limiter_state.clone(), request, next)
        }))
        // Outermost, so every layer and handler above runs inside tokio.
        .layer(TokioBridge::new(tokio))
}

/// Serves `state` on `listener` -- the one path for `main` and the
/// integration tests. Must be called inside a `rusty_tokio` runtime; request
/// bodies over `state.max_body_bytes` are answered `413`.
///
/// # Errors
///
/// When the listener's address cannot be read.
pub fn serve(
    state: AppState,
    tokio: tokio::runtime::Handle,
    listener: TcpListener,
) -> io::Result<Server> {
    let options = ServeOptions {
        max_body_bytes: state.max_body_bytes as u64,
        ..ServeOptions::default()
    };
    rusty_axum::serve(listener, None, options, build_app(state, tokio))
}
