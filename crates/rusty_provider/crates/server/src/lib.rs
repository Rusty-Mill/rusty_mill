pub mod errors;
pub mod jwt;
pub mod routes;
pub mod state;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, patch, post};
use axum::Router as AxumRouter;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::StreamableHttpService;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Builds the CORS layer from `state.cors_allowed_origins` -- `None`
/// (unset) keeps the original any-origin behavior; `Some` restricts to
/// exactly those origins. Methods/headers stay wildcard either way, since
/// this is scoped to origin restriction, not the rest of the CORS surface
/// -- and there's no credentialed (cookie-based) auth here for a wildcard
/// origin to be unsafe with. An entry that doesn't parse as a valid
/// `Origin` header value is skipped with a startup warning rather than
/// failing the whole list, same soft-failure posture as an invalid
/// `[[guardrails]]` pattern.
fn build_cors_layer(cors_allowed_origins: &Option<Vec<String>>) -> CorsLayer {
    let Some(origins) = cors_allowed_origins else {
        return CorsLayer::permissive();
    };
    let parsed: Vec<axum::http::HeaderValue> = origins
        .iter()
        .filter_map(|origin| match origin.parse() {
            Ok(value) => Some(value),
            Err(e) => {
                tracing::warn!(origin, error = %e, "skipping invalid cors_allowed_origins entry");
                None
            }
        })
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(parsed))
        .allow_methods(Any)
        .allow_headers(Any)
}

/// Mount the MCP endpoint at `state.mcp_path`, guarded by [`routes::mcp_auth`]
/// (the same `check_auth` every other route already uses -- see that
/// function's doc comment for why this isn't rusty_mcp's own OAuth 2.1
/// auth).
///
/// Uses `LocalSessionManager` rather than the newer, stateless
/// `NeverSessionManager` rusty_mcp defaults to: `NeverSessionManager`
/// rejects any client that opens with the legacy `initialize` handshake
/// instead of spec-2026-07-28's stateless `discover` bootstrap, and that
/// spec revision is barely a month old -- most MCP clients in the wild
/// today (desktop clients included) still only speak the legacy handshake.
/// `LocalSessionManager` serves both.
fn mount_mcp(router: AxumRouter<AppState>, state: &AppState) -> AxumRouter<AppState> {
    let Some(mcp) = state.mcp.clone() else {
        return router;
    };

    let factory = move || Ok((*mcp).clone());
    let service = StreamableHttpService::new(
        factory,
        Arc::new(LocalSessionManager::default()),
        Default::default(),
    );
    let guarded = AxumRouter::new()
        .fallback_service(service)
        .layer(from_fn_with_state(state.clone(), routes::mcp_auth));

    let path = state.mcp_path.trim_end_matches('/');
    if path.is_empty() {
        router.fallback_service(guarded)
    } else {
        router.nest_service(path, guarded)
    }
}

/// Builds the full axum app (routes + middleware) over the given state.
/// Shared by `main` (serving on a real listener) and integration tests
/// (serving on an ephemeral port via the same `axum::serve` path).
pub fn build_app(state: AppState) -> AxumRouter {
    let max_body_bytes = state.max_body_bytes;
    let cors_layer = build_cors_layer(&state.cors_allowed_origins);
    let router = AxumRouter::new()
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
    let router = mount_mcp(router, &state);
    router
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer)
        .layer(DefaultBodyLimit::max(max_body_bytes))
        // Outermost: a shed request costs a semaphore try-acquire and
        // nothing else -- ahead of body-limit/CORS/tracing, the same
        // "cheapest rejection first" ordering `mount_mcp`'s own guard uses.
        .layer(from_fn_with_state(state.clone(), routes::concurrency_limit))
        .with_state(state)
}
