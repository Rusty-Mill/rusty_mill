use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{stream, StreamExt};
use rp_core::{ChatRequest, EmbeddingsRequest, ModelInfo, RateLimitStatus};
use rp_router::{
    BudgetPeriod, ClientConfig, ClientRole, FreeTierStatus, ProviderStats, UsageStats,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{json_error, json_error_with_retry_after, router_error_response};
use crate::state::AppState;

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// Accepts either the legacy shared `server.api_key_env` token or any
/// configured `[[clients]]` key. Auth is skipped entirely if neither is
/// configured.
pub async fn check_auth(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    // Scoped to a block, not just an explicit `drop()` -- a std::sync
    // RwLockReadGuard held across the JWT check's `.await` below isn't
    // just bad practice, axum's Send-future requirement on handlers means
    // it wouldn't compile; a block guarantees the guard's lifetime ends
    // at `}`, which the compiler can actually see, more reliably than an
    // explicit `drop()` call does for this exact pattern.
    let (no_auth_configured, legacy_or_client_ok) = {
        let client_keys = state.client_keys.read().unwrap();
        let no_auth_configured =
            state.api_key.is_none() && client_keys.is_empty() && state.jwt.is_none();
        let ok = bearer_token(headers).is_some_and(|token| {
            state.api_key.as_deref() == Some(token) || client_keys.contains_key(token)
        });
        (no_auth_configured, ok)
    };

    if no_auth_configured {
        return None;
    }

    let Some(token) = bearer_token(headers) else {
        return Some(json_error(401, "missing or invalid API key"));
    };

    if legacy_or_client_ok {
        return None;
    }

    // A JWT that fails verification (bad signature, expired, wrong
    // issuer/audience, unreachable JWKS endpoint) falls through to the
    // same 401 below as any other bad token -- JwtVerifier::verify is
    // fail-closed by construction (see its own doc comment), so there's
    // no separate "JWT backend unavailable" case to special-case here.
    if let Some(jwt) = &state.jwt {
        if jwt.verify(token).await.is_some() {
            return None;
        }
    }

    Some(json_error(401, "missing or invalid API key"))
}

/// Enforces `server.max_concurrent_requests` (`state.concurrency_limiter`)
/// across every route, ahead of auth/rate-limiting: a `try_acquire` costs
/// nothing but a semaphore check, so a saturated server sheds load before
/// paying for anything more expensive. `None` limiter (the default, no cap
/// configured) is a no-op.
///
/// Sheds with an immediate `503` rather than queuing behind
/// `Semaphore::acquire` -- a caller waiting behind a long queue at a
/// saturated server is worse than a caller told plainly to retry.
pub async fn concurrency_limit(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(limiter) = &state.concurrency_limiter else {
        return next.run(request).await;
    };
    match Arc::clone(limiter).try_acquire_owned() {
        Ok(_permit) => next.run(request).await,
        Err(_) => json_error(503, "server is at capacity, try again shortly"),
    }
}

/// Guards the MCP endpoint (`[mcp]`) with the exact same `check_auth` every
/// other route already goes through -- deliberately not rusty_mcp's own
/// OAuth 2.1 resource-server auth, since this endpoint is mounted inside
/// this same already-authenticated app rather than run as a separate
/// listener.
pub async fn mcp_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(response) = check_auth(&state, &headers).await {
        return response;
    }
    next.run(request).await
}

/// Who a `/v1/admin/*` request is authorized as, once `check_admin_auth`
/// succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminIdentity {
    /// Authenticated with `server.admin_key_env` -- sees and manages every
    /// client, regardless of organization.
    Global,
    /// Authenticated with an admin-role client's own API key -- scoped to
    /// clients sharing this same `organization` (`None` is its own bucket,
    /// matching only other organization-less clients, not every client).
    Scoped { organization: Option<String> },
}

/// True if `/v1/admin/*` is reachable at all: `server.admin_key_env`
/// resolved, or at least one configured/provisioned client has
/// `role = "admin"`.
fn admin_api_enabled(state: &AppState) -> bool {
    state.admin_key.is_some()
        || state
            .clients
            .read()
            .unwrap()
            .iter()
            .any(|c| c.role == ClientRole::Admin)
}

/// Gates `/v1/admin/*`. Two ways in: the global `server.admin_key_env`
/// token (unscoped), or an admin-role client's own API key (scoped to its
/// `organization` -- see `AdminIdentity::Scoped`). A plain client key, or
/// any token at all when neither is configured, is rejected -- those grant
/// access to chat completions, not to other clients' spend data. Reports
/// `404` (not `401`) when the admin API isn't reachable by either path, so
/// an operator who never set either up doesn't leak that these routes
/// exist.
pub fn check_admin_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AdminIdentity, Box<Response>> {
    if !admin_api_enabled(state) {
        return Err(Box::new(json_error(404, "not found")));
    }

    let Some(token) = bearer_token(headers) else {
        return Err(Box::new(json_error(
            401,
            "missing or invalid admin API key",
        )));
    };

    if state.admin_key.as_deref() == Some(token) {
        return Ok(AdminIdentity::Global);
    }

    let scoped_client_name = state
        .client_keys
        .read()
        .unwrap()
        .get(token)
        .map(|(name, _)| name.clone());
    if let Some(name) = scoped_client_name {
        let clients = state.clients.read().unwrap();
        if let Some(client) = clients
            .iter()
            .find(|c| c.name == name && c.role == ClientRole::Admin)
        {
            return Ok(AdminIdentity::Scoped {
                organization: client.organization.clone(),
            });
        }
    }

    Err(Box::new(json_error(
        401,
        "missing or invalid admin API key",
    )))
}

/// `true` if `client` is visible to `identity` -- every client for
/// `Global`, only those sharing the same `organization` for `Scoped`.
fn in_admin_scope(identity: &AdminIdentity, client: &ClientConfig) -> bool {
    match identity {
        AdminIdentity::Global => true,
        AdminIdentity::Scoped { organization } => &client.organization == organization,
    }
}

/// Resolve which rate-limit bucket a request falls into: the identity
/// `resolve_client_identity` already resolved (a static-key or
/// JWT-claim-mapped `[[clients]]` name), or (if
/// `server.default_rate_limit_rpm` is set) a bucket keyed by source IP.
/// Returns `None` if no limit applies — an unmatched caller with no
/// configured default has no cap.
///
/// The rpm for a resolved identity always comes from `state.clients`
/// (the `ClientConfig` itself), not `state.client_keys`'s embedded rpm —
/// a JWT-mapped identity has no entry in `client_keys` at all (it never
/// authenticated via a static key), so this is the one lookup that works
/// uniformly for both paths.
///
/// The source IP is the raw TCP peer address; behind a reverse proxy this
/// is the proxy's address, not the real client's, unless you run
/// rusty_provider with the proxy's connection preserved end-to-end (this
/// router does not parse `X-Forwarded-For`, since trusting it without a
/// configured list of trusted proxies would let any caller spoof their
/// bucket).
fn resolve_rate_limit(
    state: &AppState,
    client_name: Option<&str>,
    addr: SocketAddr,
) -> Option<(String, u32)> {
    if let Some(name) = client_name {
        let rpm = state
            .clients
            .read()
            .unwrap()
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.requests_per_minute);
        if let Some(rpm) = rpm {
            return Some((format!("client:{name}"), rpm));
        }
    }
    state
        .default_rate_limit_rpm
        .map(|rpm| (format!("ip:{}", addr.ip()), rpm))
}

fn rate_limited_response(state: &AppState, identity: &str, status: &RateLimitStatus) -> Response {
    state.router.record_inbound_rate_limit_rejection(identity);
    let secs = status.retry_after_secs.ceil().max(1.0) as u64;
    let mut resp = json_error_with_retry_after(
        429,
        &format!("rate limit exceeded, retry after {secs}s"),
        Some(secs),
    );
    apply_rate_limit_headers(&mut resp, status);
    resp
}

/// Sets `X-RateLimit-Limit`/`X-RateLimit-Remaining`/`X-RateLimit-Reset` on
/// `resp` from `status` -- called on every rate-limit-checked response,
/// success or failure, so a client can see how close it is to being
/// throttled without having to wait for a `429` to find out.
/// `X-RateLimit-Reset` is seconds from now (not a Unix timestamp), same
/// convention as `Retry-After`, since this is a continuously-refilling
/// token bucket rather than a fixed window with a natural epoch boundary.
fn apply_rate_limit_headers(resp: &mut Response, status: &RateLimitStatus) {
    let headers = resp.headers_mut();
    for (name, value) in [
        ("x-ratelimit-limit", status.limit.to_string()),
        ("x-ratelimit-remaining", status.remaining.to_string()),
        (
            "x-ratelimit-reset",
            status.reset_secs.ceil().max(0.0).to_string(),
        ),
    ] {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }
}

/// The `[[clients]]` name this request authenticates as, if any. Tried in
/// order:
/// 1. The bearer token directly matches a configured client key (the
///    existing, static-key path).
/// 2. If `[jwt].client_claim` is configured, the token verifies as a JWT
///    (re-verified here -- `check_auth` already checked it once for the
///    authentication decision, but discards the claims, so this is a
///    second, cheap decode/signature-check, not a second *auth* check:
///    `check_auth` already ran first and would have rejected an invalid
///    token before this function is ever reached) and that claim's string
///    value matches a configured client's `name`.
///
/// `None` for an unauthenticated request, one using only the shared
/// `server.api_key_env` token, a JWT with no configured/matching claim, or
/// any other unmatched caller — spend budgets and per-subject rate limits
/// only apply to a resolved identity, never the IP-bucketed fallback.
async fn resolve_client_identity(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let token = bearer_token(headers)?;
    if let Some((name, _)) = state.client_keys.read().unwrap().get(token) {
        return Some(name.clone());
    }
    let jwt = state.jwt.as_ref()?;
    let claim_name = jwt.client_claim()?;
    let claims = jwt.verify(token).await?;
    let claim_value = crate::jwt::claim_as_str(&claims, claim_name)?;
    state
        .clients
        .read()
        .unwrap()
        .iter()
        .find(|c| c.name == claim_value)
        .map(|c| c.name.clone())
}

fn budget_exceeded_response(
    state: &AppState,
    client_name: &str,
    exceeded: rp_router::ClientBudgetExceeded,
) -> Response {
    state.router.record_client_budget_rejection(client_name);
    json_error(
        402,
        &format!(
            "client \"{client_name}\" has exceeded its configured budget (${:.2} spent of ${:.2})",
            exceeded.spent_usd, exceeded.budget_usd
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::http::HeaderValue;
    use rp_core::RateLimiter;
    use rp_router::{Config, Router};

    use crate::jwt::JwtVerifier;

    async fn test_state(
        client_keys: Vec<(&str, &str, u32)>,
        default_rate_limit_rpm: Option<u32>,
    ) -> AppState {
        test_state_with_jwt(client_keys, default_rate_limit_rpm, None).await
    }

    // `clients` is always kept in sync with `client_keys` here, mirroring
    // the invariant every admin create/update/delete handler maintains in
    // production (`resolve_rate_limit` sources rpm from `clients`, not the
    // rpm embedded in `client_keys`, so a test with an empty `clients` vec
    // but a non-empty `client_keys` map would silently fall through to the
    // IP-bucket default instead of the client bucket it looks like it's
    // testing).
    async fn test_state_with_jwt(
        client_keys: Vec<(&str, &str, u32)>,
        default_rate_limit_rpm: Option<u32>,
        jwt: Option<Arc<JwtVerifier>>,
    ) -> AppState {
        let router =
            Arc::new(Router::from_config(&Config::from_toml_str("providers = {}").unwrap()).await);
        let clients = client_keys
            .iter()
            .map(|(_, name, rpm)| ClientConfig {
                name: name.to_string(),
                api_key_env: format!("{name}_KEY"),
                requests_per_minute: *rpm,
                budget_usd: None,
                budget_period: BudgetPeriod::default(),
                organization: None,
                workspace: None,
                role: ClientRole::Member,
            })
            .collect::<Vec<_>>();
        let client_keys = client_keys
            .into_iter()
            .map(|(key, name, rpm)| (key.to_string(), (name.to_string(), rpm)))
            .collect::<HashMap<_, _>>();
        AppState {
            router,
            api_key: None,
            client_keys: Arc::new(std::sync::RwLock::new(client_keys)),
            default_rate_limit_rpm,
            rate_limiter: Arc::new(RateLimiter::new()),
            clients: Arc::new(std::sync::RwLock::new(clients)),
            admin_key: None,
            max_body_bytes: 20 * 1024 * 1024,
            jwt,
            mcp: None,
            mcp_path: "/mcp".to_string(),
            concurrency_limiter: None,
        }
    }

    fn bearer_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    fn addr() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 54321))
    }

    // --- resolve_rate_limit ----------------------------------------------------

    #[tokio::test]
    async fn resolve_rate_limit_is_none_with_no_client_match_and_no_default() {
        let state = test_state(vec![], None).await;
        let result = resolve_rate_limit(&state, None, addr());
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn resolve_rate_limit_falls_back_to_ip_bucket_when_default_is_configured() {
        let state = test_state(vec![], Some(60)).await;
        let result = resolve_rate_limit(&state, None, addr());
        assert_eq!(result, Some(("ip:127.0.0.1".to_string(), 60)));
    }

    #[tokio::test]
    async fn resolve_rate_limit_uses_client_bucket_when_an_identity_is_resolved() {
        let state = test_state(vec![("secret-key", "acme", 30)], None).await;
        let result = resolve_rate_limit(&state, Some("acme"), addr());
        assert_eq!(result, Some(("client:acme".to_string(), 30)));
    }

    #[tokio::test]
    async fn resolve_rate_limit_prefers_client_bucket_over_ip_fallback() {
        let state = test_state(vec![("secret-key", "acme", 30)], Some(60)).await;
        let result = resolve_rate_limit(&state, Some("acme"), addr());
        assert_eq!(
            result,
            Some(("client:acme".to_string(), 30)),
            "a resolved client identity must win over the IP-bucket default"
        );
    }

    #[tokio::test]
    async fn resolve_rate_limit_falls_back_to_ip_when_no_identity_was_resolved() {
        let state = test_state(vec![("secret-key", "acme", 30)], Some(60)).await;
        let result = resolve_rate_limit(&state, None, addr());
        assert_eq!(result, Some(("ip:127.0.0.1".to_string(), 60)));
    }

    #[tokio::test]
    async fn resolve_rate_limit_is_none_when_no_identity_was_resolved_and_no_default() {
        let state = test_state(vec![("secret-key", "acme", 30)], None).await;
        let result = resolve_rate_limit(&state, None, addr());
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn resolve_rate_limit_is_none_when_the_resolved_identity_has_no_matching_client() {
        // Defensive: `client_name` came from `resolve_client_identity`, which
        // only ever returns a name backed by a real `[[clients]]` entry, but
        // `resolve_rate_limit` itself doesn't assume that -- an identity with
        // no matching `ClientConfig` falls back to the IP bucket rather than
        // panicking or fabricating a limit.
        let state = test_state(vec![], Some(60)).await;
        let result = resolve_rate_limit(&state, Some("ghost"), addr());
        assert_eq!(result, Some(("ip:127.0.0.1".to_string(), 60)));
    }

    #[tokio::test]
    async fn resolve_rate_limit_ip_bucket_key_reflects_the_caller_address() {
        let state = test_state(vec![], Some(60)).await;
        let other_addr = SocketAddr::from(([10, 0, 0, 5], 8080));
        let result = resolve_rate_limit(&state, None, other_addr);
        assert_eq!(result, Some(("ip:10.0.0.5".to_string(), 60)));
    }

    // --- rate_limited_response ---------------------------------------------------

    fn status(
        limit: u32,
        remaining: u32,
        retry_after_secs: f64,
        reset_secs: f64,
    ) -> RateLimitStatus {
        RateLimitStatus {
            limit,
            remaining,
            retry_after_secs,
            reset_secs,
        }
    }

    #[tokio::test]
    async fn rate_limited_response_returns_429_with_a_rounded_up_retry_after_header() {
        let state = test_state(vec![], None).await;
        let resp = rate_limited_response(&state, "ip:127.0.0.1", &status(60, 0, 0.2, 0.2));
        assert_eq!(resp.status(), 429);
        assert_eq!(
            resp.headers().get("retry-after").unwrap(),
            &HeaderValue::from_static("1"),
            "0.2s should round up to a minimum of 1s"
        );
    }

    #[tokio::test]
    async fn rate_limited_response_retry_after_ceils_fractional_seconds() {
        let state = test_state(vec![], None).await;
        let resp = rate_limited_response(&state, "ip:127.0.0.1", &status(60, 0, 4.1, 4.1));
        assert_eq!(
            resp.headers().get("retry-after").unwrap(),
            &HeaderValue::from_static("5")
        );
    }

    #[tokio::test]
    async fn rate_limited_response_sets_x_ratelimit_headers() {
        let state = test_state(vec![], None).await;
        let resp = rate_limited_response(&state, "ip:127.0.0.1", &status(60, 0, 4.1, 4.6));
        assert_eq!(
            resp.headers().get("x-ratelimit-limit").unwrap(),
            &HeaderValue::from_static("60")
        );
        assert_eq!(
            resp.headers().get("x-ratelimit-remaining").unwrap(),
            &HeaderValue::from_static("0")
        );
        assert_eq!(
            resp.headers().get("x-ratelimit-reset").unwrap(),
            &HeaderValue::from_static("5"),
            "reset_secs is ceil'd, same rounding convention as retry-after"
        );
    }

    #[tokio::test]
    async fn rate_limited_response_records_the_rejection_under_the_given_identity() {
        let state = test_state(vec![], None).await;
        rate_limited_response(&state, "client:acme", &status(60, 0, 1.0, 1.0));
        let metrics = state.router.render_prometheus_metrics();
        assert!(metrics.contains("rusty_provider_inbound_rate_limit_rejections_total"));
        assert!(metrics.contains(r#"identity="client:acme""#));
    }

    #[tokio::test]
    async fn rate_limited_response_body_reports_the_rounded_retry_after_in_the_message() {
        let state = test_state(vec![], None).await;
        let resp = rate_limited_response(&state, "ip:127.0.0.1", &status(60, 0, 4.1, 4.1));
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], 429);
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("retry after 5s"));
    }

    // --- resolve_client_identity -----------------------------------------

    #[tokio::test]
    async fn resolve_client_identity_is_none_with_no_bearer_token() {
        let state = test_state(vec![("secret-key", "acme", 30)], None).await;
        assert_eq!(
            resolve_client_identity(&state, &HeaderMap::new()).await,
            None
        );
    }

    #[tokio::test]
    async fn resolve_client_identity_is_none_for_an_unmatched_token() {
        let state = test_state(vec![("secret-key", "acme", 30)], None).await;
        assert_eq!(
            resolve_client_identity(&state, &bearer_headers("wrong-key")).await,
            None
        );
    }

    #[tokio::test]
    async fn resolve_client_identity_returns_the_name_for_a_matching_client_token() {
        let state = test_state(vec![("secret-key", "acme", 30)], None).await;
        assert_eq!(
            resolve_client_identity(&state, &bearer_headers("secret-key")).await,
            Some("acme".to_string())
        );
    }

    // --- resolve_client_identity (JWT claim mapping) ----------------------

    fn hs256_jwt_config(client_claim: Option<&str>) -> rp_router::JwtConfig {
        rp_router::JwtConfig {
            jwks_url: None,
            hs256_secret_env: Some("UNUSED_IN_TESTS".to_string()),
            issuer: None,
            audience: None,
            jwks_cache_secs: 300,
            client_claim: client_claim.map(str::to_string),
        }
    }

    fn hs256_jwt(secret: &str, claims: &serde_json::Value) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    fn future_exp() -> i64 {
        (std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[tokio::test]
    async fn resolve_client_identity_maps_a_jwt_claim_to_a_matching_client() {
        let cfg = hs256_jwt_config(Some("sub"));
        let jwt = Arc::new(JwtVerifier::new(&cfg, Some("jwt-secret".to_string())).unwrap());
        let state = test_state_with_jwt(vec![("secret-key", "acme", 30)], None, Some(jwt)).await;
        let token = hs256_jwt(
            "jwt-secret",
            &serde_json::json!({"sub": "acme", "exp": future_exp()}),
        );
        assert_eq!(
            resolve_client_identity(&state, &bearer_headers(&token)).await,
            Some("acme".to_string())
        );
    }

    #[tokio::test]
    async fn resolve_client_identity_is_none_when_the_jwt_claim_matches_no_client() {
        let cfg = hs256_jwt_config(Some("sub"));
        let jwt = Arc::new(JwtVerifier::new(&cfg, Some("jwt-secret".to_string())).unwrap());
        let state = test_state_with_jwt(vec![("secret-key", "acme", 30)], None, Some(jwt)).await;
        let token = hs256_jwt(
            "jwt-secret",
            &serde_json::json!({"sub": "someone-else", "exp": future_exp()}),
        );
        assert_eq!(
            resolve_client_identity(&state, &bearer_headers(&token)).await,
            None
        );
    }

    #[tokio::test]
    async fn resolve_client_identity_is_none_when_client_claim_is_not_configured() {
        // `[jwt].client_claim` unset (the default) -- a valid JWT whose
        // `sub` happens to match a client name still resolves to no
        // identity, preserving pre-mapping behavior for anyone not opted in.
        let cfg = hs256_jwt_config(None);
        let jwt = Arc::new(JwtVerifier::new(&cfg, Some("jwt-secret".to_string())).unwrap());
        let state = test_state_with_jwt(vec![("secret-key", "acme", 30)], None, Some(jwt)).await;
        let token = hs256_jwt(
            "jwt-secret",
            &serde_json::json!({"sub": "acme", "exp": future_exp()}),
        );
        assert_eq!(
            resolve_client_identity(&state, &bearer_headers(&token)).await,
            None
        );
    }

    #[tokio::test]
    async fn resolve_client_identity_is_none_when_the_configured_claim_is_absent() {
        let cfg = hs256_jwt_config(Some("email"));
        let jwt = Arc::new(JwtVerifier::new(&cfg, Some("jwt-secret".to_string())).unwrap());
        let state = test_state_with_jwt(vec![("secret-key", "acme", 30)], None, Some(jwt)).await;
        let token = hs256_jwt(
            "jwt-secret",
            &serde_json::json!({"sub": "acme", "exp": future_exp()}),
        );
        assert_eq!(
            resolve_client_identity(&state, &bearer_headers(&token)).await,
            None
        );
    }

    #[tokio::test]
    async fn resolve_client_identity_prefers_a_static_key_match_over_jwt_mapping() {
        let cfg = hs256_jwt_config(Some("sub"));
        let jwt = Arc::new(JwtVerifier::new(&cfg, Some("jwt-secret".to_string())).unwrap());
        let state = test_state_with_jwt(vec![("secret-key", "acme", 30)], None, Some(jwt)).await;
        // The bearer token itself is a configured static key, so it's
        // resolved directly -- never handed to jwt.verify() at all (a
        // static key is never a well-formed JWT here, so this also proves
        // the static-key branch short-circuits before attempting to decode
        // it as one).
        assert_eq!(
            resolve_client_identity(&state, &bearer_headers("secret-key")).await,
            Some("acme".to_string())
        );
    }

    fn client_config(name: &str, organization: Option<&str>, role: ClientRole) -> ClientConfig {
        ClientConfig {
            name: name.to_string(),
            api_key_env: String::new(),
            requests_per_minute: 30,
            budget_usd: None,
            budget_period: BudgetPeriod::default(),
            organization: organization.map(str::to_string),
            workspace: None,
            role,
        }
    }

    // --- check_admin_auth ------------------------------------------------------

    #[tokio::test]
    async fn check_admin_auth_is_404_when_admin_key_is_not_configured() {
        let state = test_state(vec![], None).await;
        let resp = check_admin_auth(&state, &HeaderMap::new()).unwrap_err();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn check_admin_auth_is_401_with_no_bearer_token_when_configured() {
        let state = AppState {
            admin_key: Some("admin-secret".to_string()),
            ..test_state(vec![], None).await
        };
        let resp = check_admin_auth(&state, &HeaderMap::new()).unwrap_err();
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn check_admin_auth_is_401_with_a_wrong_token() {
        let state = AppState {
            admin_key: Some("admin-secret".to_string()),
            ..test_state(vec![], None).await
        };
        let resp = check_admin_auth(&state, &bearer_headers("wrong")).unwrap_err();
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn check_admin_auth_rejects_a_regular_client_key() {
        // A client key that authenticates chat completions must not also
        // unlock the admin API -- they're deliberately separate trust
        // levels, unless the client is explicitly given `role = "admin"`.
        let state = AppState {
            admin_key: Some("admin-secret".to_string()),
            clients: Arc::new(std::sync::RwLock::new(vec![client_config(
                "acme",
                None,
                ClientRole::Member,
            )])),
            ..test_state(vec![("client-key", "acme", 30)], None).await
        };
        let resp = check_admin_auth(&state, &bearer_headers("client-key")).unwrap_err();
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn check_admin_auth_accepts_an_admin_role_clients_own_key_as_scoped() {
        let state = AppState {
            clients: Arc::new(std::sync::RwLock::new(vec![client_config(
                "acme",
                Some("acme-corp"),
                ClientRole::Admin,
            )])),
            ..test_state(vec![("client-key", "acme", 30)], None).await
        };
        let identity = check_admin_auth(&state, &bearer_headers("client-key")).unwrap();
        assert_eq!(
            identity,
            AdminIdentity::Scoped {
                organization: Some("acme-corp".to_string())
            }
        );
    }

    #[tokio::test]
    async fn check_admin_auth_is_reachable_via_an_admin_role_client_with_no_global_key_configured()
    {
        // admin_api_enabled must consider role="admin" clients even when
        // server.admin_key_env was never set -- otherwise this would 404
        // instead of authenticating.
        let state = AppState {
            clients: Arc::new(std::sync::RwLock::new(vec![client_config(
                "acme",
                None,
                ClientRole::Admin,
            )])),
            ..test_state(vec![("client-key", "acme", 30)], None).await
        };
        assert!(check_admin_auth(&state, &bearer_headers("client-key")).is_ok());
    }

    // --- audit_identity_fields -------------------------------------------

    #[test]
    fn audit_identity_fields_reports_global_with_no_organization() {
        assert_eq!(
            audit_identity_fields(&AdminIdentity::Global),
            ("global", "")
        );
    }

    #[test]
    fn audit_identity_fields_reports_scoped_with_its_organization() {
        let identity = AdminIdentity::Scoped {
            organization: Some("acme-corp".to_string()),
        };
        assert_eq!(audit_identity_fields(&identity), ("scoped", "acme-corp"));
    }

    #[test]
    fn audit_identity_fields_reports_scoped_with_no_organization_as_empty() {
        let identity = AdminIdentity::Scoped { organization: None };
        assert_eq!(audit_identity_fields(&identity), ("scoped", ""));
    }

    #[tokio::test]
    async fn check_admin_auth_passes_with_the_correct_token() {
        let state = AppState {
            admin_key: Some("admin-secret".to_string()),
            ..test_state(vec![], None).await
        };
        assert!(check_admin_auth(&state, &bearer_headers("admin-secret")).is_ok());
    }

    // --- budget_exceeded_response ----------------------------------------------

    #[tokio::test]
    async fn budget_exceeded_response_returns_402() {
        let state = test_state(vec![], None).await;
        let resp = budget_exceeded_response(
            &state,
            "acme",
            rp_router::ClientBudgetExceeded {
                spent_usd: 12.5,
                budget_usd: 10.0,
            },
        );
        assert_eq!(resp.status(), 402);
    }

    #[tokio::test]
    async fn budget_exceeded_response_records_the_rejection_under_the_client_name() {
        let state = test_state(vec![], None).await;
        budget_exceeded_response(
            &state,
            "acme",
            rp_router::ClientBudgetExceeded {
                spent_usd: 12.5,
                budget_usd: 10.0,
            },
        );
        let metrics = state.router.render_prometheus_metrics();
        assert!(metrics.contains("rusty_provider_client_budget_rejections_total"));
        assert!(metrics.contains(r#"client="acme""#));
    }

    #[tokio::test]
    async fn budget_exceeded_response_body_reports_the_client_and_amounts() {
        let state = test_state(vec![], None).await;
        let resp = budget_exceeded_response(
            &state,
            "acme",
            rp_router::ClientBudgetExceeded {
                spent_usd: 12.5,
                budget_usd: 10.0,
            },
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], 402);
        let message = json["error"]["message"].as_str().unwrap();
        assert!(message.contains("acme"));
        assert!(message.contains("12.50"));
        assert!(message.contains("10.00"));
    }
}

/// A single self-contained static page (no build step, no JS framework, no
/// CDN dependency) that authenticates and renders entirely client-side --
/// it prompts for a bearer token and attaches it to `fetch()` calls against
/// the JSON endpoints this file already exposes (`/v1/models`, `/v1/usage`,
/// `/v1/providers/stats`, `/v1/free-tiers`, `/v1/admin/clients*`), so it's
/// subject to exactly the same `check_auth`/`check_admin_auth` rules those
/// already enforce. The page itself carries no secrets and needs none of
/// its own, so it's served unauthenticated -- same reasoning as `/health`.
pub async fn dashboard() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../assets/dashboard.html"),
    )
        .into_response()
}

pub async fn health() -> &'static str {
    "ok"
}

/// Readiness check, distinct from `health` above: `health` only confirms
/// the process is up, while this confirms it can actually serve traffic
/// right now -- currently just "is `[persistence]`, if configured,
/// actually reachable." `200 {"status": "ready"}` when it is (or nothing
/// external is configured to check); `503` with the failure reason when
/// it isn't.
pub async fn ready(State(state): State<AppState>) -> Response {
    match state.router.check_readiness().await {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response(),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "not ready", "reason": reason })),
        )
            .into_response(),
    }
}

pub async fn list_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    let data: Vec<ModelInfo> = state
        .router
        .route_aliases()
        .map(|alias| ModelInfo {
            id: alias.to_string(),
            object: "model",
            owned_by: "router-alias".to_string(),
            context_length: None,
            pricing: None,
            supported_params: None,
        })
        .chain(state.router.configured_providers().map(|p| ModelInfo {
            id: format!("{p}/*"),
            object: "model",
            owned_by: p.to_string(),
            context_length: None,
            pricing: None,
            supported_params: None,
        }))
        .chain(state.router.priced_models())
        .collect();

    Json(json!({ "object": "list", "data": data })).into_response()
}

#[derive(Serialize)]
struct UsageEntry {
    model: String,
    #[serde(flatten)]
    stats: UsageStats,
}

pub async fn usage_stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    let data: Vec<UsageEntry> = state
        .router
        .usage_snapshot()
        .await
        .into_iter()
        .map(|(model, stats)| UsageEntry { model, stats })
        .collect();

    Json(json!({ "object": "list", "data": data })).into_response()
}

#[derive(Serialize)]
struct FreeTierEntry {
    model: String,
    #[serde(flatten)]
    status: FreeTierStatus,
}

/// Operator-declared free-token budgets (`[[free_tiers]]`) vs. this
/// process's tracked usage against them -- see the README's "Free tiers"
/// section. JSON-only per ADR-0002; empty `data` when no `[[free_tiers]]`
/// entries are configured, same "nothing to report" shape every other
/// list endpoint here uses.
pub async fn free_tiers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    let data: Vec<FreeTierEntry> = state
        .router
        .free_tier_status()
        .into_iter()
        .map(|(model, status)| FreeTierEntry { model, status })
        .collect();

    Json(json!({ "object": "list", "data": data })).into_response()
}

#[derive(Serialize)]
struct ProviderStatsEntry {
    model: String,
    #[serde(flatten)]
    stats: ProviderStats,
}

pub async fn provider_stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    let data: Vec<ProviderStatsEntry> = state
        .router
        .provider_stats()
        .into_iter()
        .map(|(model, stats)| ProviderStatsEntry { model, stats })
        .collect();

    Json(json!({ "object": "list", "data": data })).into_response()
}

#[derive(Deserialize)]
pub struct GenerationQuery {
    id: String,
}

pub async fn generation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GenerationQuery>,
) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    match state.router.generation(&query.id) {
        Some(record) => Json(record).into_response(),
        None => json_error(404, "no generation found for that id"),
    }
}

pub async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        state.router.render_prometheus_metrics(),
    )
        .into_response()
}

#[derive(Serialize)]
struct AdminClientEntry {
    name: String,
    organization: Option<String>,
    workspace: Option<String>,
    role: ClientRole,
    requests_per_minute: u32,
    budget_usd: Option<f64>,
    budget_period: Option<rp_router::BudgetPeriod>,
    /// The client's live tracked spend for the current `budget_period`, or
    /// `None` for a client with no `budget_usd` configured -- there's
    /// nothing to track.
    spent_usd: Option<f64>,
}

pub async fn admin_list_clients(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let identity = match check_admin_auth(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return *resp,
    };

    // Clone out of the lock before the `.await` loop below -- a std
    // `RwLockReadGuard` held across an await point would make this
    // future non-`Send`, which axum handlers must be.
    let clients: Vec<_> = state
        .clients
        .read()
        .unwrap()
        .iter()
        .filter(|c| in_admin_scope(&identity, c))
        .cloned()
        .collect();
    let mut data = Vec::with_capacity(clients.len());
    for client in &clients {
        let status = state.router.client_spend_status(&client.name).await;
        data.push(AdminClientEntry {
            name: client.name.clone(),
            organization: client.organization.clone(),
            workspace: client.workspace.clone(),
            role: client.role,
            requests_per_minute: client.requests_per_minute,
            budget_usd: client.budget_usd,
            budget_period: status.map(|s| s.period),
            spent_usd: status.map(|s| s.spent_usd),
        });
    }

    Json(json!({ "object": "list", "data": data })).into_response()
}

#[derive(Serialize)]
struct OrganizationClientEntry {
    name: String,
    workspace: Option<String>,
    role: ClientRole,
    requests_per_minute: u32,
    budget_usd: Option<f64>,
    spent_usd: Option<f64>,
}

#[derive(Serialize)]
struct OrganizationEntry {
    organization: Option<String>,
    clients: Vec<OrganizationClientEntry>,
}

/// Rolls up every client `identity` can see into `(organization,
/// [workspace-tagged clients])` groups -- a `Global` caller gets one group
/// per distinct `organization` value (including `null` for organization-
/// less clients); a `Scoped` caller only ever gets its own single group,
/// since `in_admin_scope` already filtered to just that organization.
pub async fn admin_list_organizations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let identity = match check_admin_auth(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return *resp,
    };

    let clients: Vec<_> = state
        .clients
        .read()
        .unwrap()
        .iter()
        .filter(|c| in_admin_scope(&identity, c))
        .cloned()
        .collect();

    let mut groups: Vec<(Option<String>, Vec<ClientConfig>)> = Vec::new();
    for client in clients {
        match groups
            .iter_mut()
            .find(|(org, _)| org == &client.organization)
        {
            Some((_, members)) => members.push(client),
            None => groups.push((client.organization.clone(), vec![client])),
        }
    }

    let mut data = Vec::with_capacity(groups.len());
    for (organization, members) in groups {
        let mut entries = Vec::with_capacity(members.len());
        for client in &members {
            let status = state.router.client_spend_status(&client.name).await;
            entries.push(OrganizationClientEntry {
                name: client.name.clone(),
                workspace: client.workspace.clone(),
                role: client.role,
                requests_per_minute: client.requests_per_minute,
                budget_usd: client.budget_usd,
                spent_usd: status.map(|s| s.spent_usd),
            });
        }
        data.push(OrganizationEntry {
            organization,
            clients: entries,
        });
    }

    Json(json!({ "object": "list", "data": data })).into_response()
}

pub async fn admin_reset_client_spend(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    let identity = match check_admin_auth(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return *resp,
    };

    let in_scope = state
        .clients
        .read()
        .unwrap()
        .iter()
        .any(|c| c.name == name && in_admin_scope(&identity, c));
    if !in_scope {
        return json_error(
            404,
            &format!("no client named \"{name}\" with a configured budget"),
        );
    }

    if state.router.reset_client_spend(&name) {
        admin_audit_log(&identity, "reset_client_spend", &name);
        Json(json!({ "status": "ok" })).into_response()
    } else {
        json_error(
            404,
            &format!("no client named \"{name}\" with a configured budget"),
        )
    }
}

/// Days of history `admin_client_usage_history` returns when `days` is
/// omitted from the query string.
const DEFAULT_USAGE_HISTORY_DAYS: u32 = 30;
/// Upper bound on `days`, regardless of what the caller asks for -- an
/// unbounded range would let one request force an unbounded table scan.
const MAX_USAGE_HISTORY_DAYS: u32 = 90;

#[derive(Deserialize)]
pub struct UsageHistoryQuery {
    #[serde(default)]
    days: Option<u32>,
}

/// `GET /v1/admin/clients/{name}/usage-history` -- day-bucketed
/// requests/tokens/cost for `name`, from `[persistence]`'s
/// `client_daily_usage` table. Unlike `admin_reset_client_spend` and the
/// rest of this file's budget endpoints, this isn't scoped to clients
/// with a configured `budget_usd` -- `Router::client_usage_history`
/// tracks every named client, so any client visible to `identity` (see
/// `in_admin_scope`) is queryable here, budgeted or not.
pub async fn admin_client_usage_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(query): Query<UsageHistoryQuery>,
) -> Response {
    let identity = match check_admin_auth(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return *resp,
    };

    let in_scope = state
        .clients
        .read()
        .unwrap()
        .iter()
        .any(|c| c.name == name && in_admin_scope(&identity, c));
    if !in_scope {
        return json_error(404, &format!("no client named \"{name}\""));
    }

    let days = query
        .days
        .unwrap_or(DEFAULT_USAGE_HISTORY_DAYS)
        .clamp(1, MAX_USAGE_HISTORY_DAYS);
    let history = state.router.client_usage_history(&name, days).await;

    Json(json!({ "object": "list", "client": name, "days": days, "data": history })).into_response()
}

/// Structured audit line for an admin-API mutation -- otherwise there's no
/// way to answer "who changed this client's budget, and when" after the
/// fact beyond whatever's in general application logs (if even that).
/// `identity` distinguishes the global admin token from a scoped
/// admin-role client's own key (see [`AdminIdentity`]); `target` is the
/// client name being acted on. One line per successful mutation, emitted
/// only after the action has actually taken effect -- a rejected/no-op
/// request (404, 409, validation error) isn't logged here, since nothing
/// changed for it to record.
fn admin_audit_log(identity: &AdminIdentity, action: &str, target: &str) {
    let (identity_label, organization) = audit_identity_fields(identity);
    tracing::info!(
        identity = identity_label,
        organization,
        action,
        target,
        "admin action"
    );
}

/// The `(identity, organization)` fields `admin_audit_log` attaches to its
/// event -- split out from the `tracing::info!` call itself so the
/// per-identity-variant logic is unit-testable without a tracing
/// subscriber to capture against.
fn audit_identity_fields(identity: &AdminIdentity) -> (&'static str, &str) {
    match identity {
        AdminIdentity::Global => ("global", ""),
        AdminIdentity::Scoped { organization } => ("scoped", organization.as_deref().unwrap_or("")),
    }
}

/// Random 64-character hex token (32 bytes of CSPRNG output via `ring`,
/// already a transitive dependency through `rustls`) for a
/// runtime-provisioned client's API key, prefixed for recognizability the
/// same way GitHub/Stripe-style tokens are.
fn generate_api_key() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system RNG should not fail");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("rp_{hex}")
}

#[derive(Deserialize)]
pub struct CreateClientRequest {
    name: String,
    requests_per_minute: u32,
    #[serde(default)]
    budget_usd: Option<f64>,
    #[serde(default)]
    budget_period: BudgetPeriod,
    /// Explicit API key value to assign. If omitted, the server generates
    /// a random one and returns it in the response -- the only time it's
    /// ever shown, the same hygiene as GitHub/Stripe-style API keys.
    #[serde(default)]
    api_key: Option<String>,
    /// A `Global` caller may set this to any value (or leave it unset for
    /// no organization). A `Scoped` caller (an admin-role client's own
    /// key) always has it forced to their own organization, regardless of
    /// what's sent here -- creating a client outside your own organization
    /// isn't a thing a scoped admin can do.
    #[serde(default)]
    organization: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    role: ClientRole,
}

#[derive(Serialize)]
struct ClientProvisionResponse {
    name: String,
    organization: Option<String>,
    workspace: Option<String>,
    role: ClientRole,
    requests_per_minute: u32,
    budget_usd: Option<f64>,
    budget_period: BudgetPeriod,
    /// Only present when this response is the one time the key is shown:
    /// creation, or an update that rotated it.
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
}

pub async fn admin_create_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let identity = match check_admin_auth(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return *resp,
    };
    // Deserialized only after the auth check above -- unlike a `Json<T>`
    // extractor parameter, which axum would run before the handler body
    // (and thus before `check_admin_auth`) even executes, leaking a 415 on
    // a malformed/missing body to an unauthenticated caller instead of the
    // 401/404 they should see.
    let mut req: CreateClientRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => return json_error(400, &format!("invalid request body: {e}")),
    };
    if req.name.is_empty() {
        return json_error(400, "\"name\" must not be empty");
    }
    if req.requests_per_minute == 0 {
        return json_error(400, "\"requests_per_minute\" must be greater than zero");
    }
    if req.budget_usd.is_some_and(|b| b < 0.0) {
        return json_error(400, "\"budget_usd\" must not be negative");
    }
    if let AdminIdentity::Scoped { organization } = &identity {
        req.organization = organization.clone();
    }

    {
        let clients = state.clients.read().unwrap();
        if clients.iter().any(|c| c.name == req.name) {
            return json_error(
                409,
                &format!("a client named \"{}\" already exists", req.name),
            );
        }
    }

    let api_key = req.api_key.unwrap_or_else(generate_api_key);
    {
        let mut keys = state.client_keys.write().unwrap();
        if keys.contains_key(&api_key) {
            return json_error(409, "a client with this API key already exists");
        }
        keys.insert(api_key.clone(), (req.name.clone(), req.requests_per_minute));
    }
    state.clients.write().unwrap().push(ClientConfig {
        name: req.name.clone(),
        // Runtime-provisioned clients hold their key directly in
        // `client_keys` rather than resolving one from an env var --
        // there's no env var to name here.
        api_key_env: String::new(),
        requests_per_minute: req.requests_per_minute,
        budget_usd: req.budget_usd,
        budget_period: req.budget_period,
        organization: req.organization.clone(),
        workspace: req.workspace.clone(),
        role: req.role,
    });
    state.router.set_client_budget(
        &req.name,
        req.budget_usd
            .map(|budget_usd| (budget_usd, req.budget_period)),
    );
    admin_audit_log(&identity, "create_client", &req.name);

    (
        StatusCode::CREATED,
        Json(ClientProvisionResponse {
            name: req.name,
            organization: req.organization,
            workspace: req.workspace,
            role: req.role,
            requests_per_minute: req.requests_per_minute,
            budget_usd: req.budget_usd,
            budget_period: req.budget_period,
            api_key: Some(api_key),
        }),
    )
        .into_response()
}

/// Distinguishes "field omitted" (`None`, leave alone) from "field
/// explicitly set to `null`" (`Some(None)`, clear it) for `budget_usd` --
/// the standard serde workaround, since `#[serde(default)]` alone can't
/// tell those two cases apart for a plain `Option<T>` field.
fn deserialize_present_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::deserialize(deserializer).map(Some)
}

#[derive(Deserialize, Default)]
pub struct UpdateClientRequest {
    #[serde(default)]
    requests_per_minute: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    budget_usd: Option<Option<f64>>,
    #[serde(default)]
    budget_period: Option<BudgetPeriod>,
    /// If `true`, revokes the client's current API key (if any) and
    /// issues a new one, returned in the response. Otherwise the existing
    /// key (if any) keeps working unchanged.
    #[serde(default)]
    rotate_api_key: bool,
}

pub async fn admin_update_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let identity = match check_admin_auth(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return *resp,
    };
    // See `admin_create_client` for why this is deserialized manually
    // after the auth check, rather than via a `Json<T>` extractor
    // parameter.
    let req: UpdateClientRequest = if body.is_empty() {
        UpdateClientRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(req) => req,
            Err(e) => return json_error(400, &format!("invalid request body: {e}")),
        }
    };
    if req.requests_per_minute == Some(0) {
        return json_error(400, "\"requests_per_minute\" must be greater than zero");
    }
    if let Some(Some(budget_usd)) = req.budget_usd {
        if budget_usd < 0.0 {
            return json_error(400, "\"budget_usd\" must not be negative");
        }
    }

    let updated = {
        let mut clients = state.clients.write().unwrap();
        let Some(client) = clients
            .iter_mut()
            .find(|c| c.name == name && in_admin_scope(&identity, c))
        else {
            return json_error(404, &format!("no client named \"{name}\""));
        };
        if let Some(rpm) = req.requests_per_minute {
            client.requests_per_minute = rpm;
        }
        if let Some(budget_usd) = req.budget_usd {
            client.budget_usd = budget_usd;
        }
        if let Some(period) = req.budget_period {
            client.budget_period = period;
        }
        client.clone()
    };

    let mut new_api_key = None;
    {
        let mut keys = state.client_keys.write().unwrap();
        let existing_key = keys
            .iter()
            .find(|(_, (n, _))| n == &name)
            .map(|(k, _)| k.clone());
        if req.rotate_api_key {
            if let Some(old_key) = &existing_key {
                keys.remove(old_key);
            }
            let key = generate_api_key();
            keys.insert(key.clone(), (name.clone(), updated.requests_per_minute));
            new_api_key = Some(key);
        } else if let Some(old_key) = existing_key {
            if let Some(entry) = keys.get_mut(&old_key) {
                entry.1 = updated.requests_per_minute;
            }
        }
    }

    state.router.set_client_budget(
        &name,
        updated
            .budget_usd
            .map(|budget_usd| (budget_usd, updated.budget_period)),
    );
    admin_audit_log(&identity, "update_client", &name);

    Json(ClientProvisionResponse {
        name: updated.name,
        organization: updated.organization,
        workspace: updated.workspace,
        role: updated.role,
        requests_per_minute: updated.requests_per_minute,
        budget_usd: updated.budget_usd,
        budget_period: updated.budget_period,
        api_key: new_api_key,
    })
    .into_response()
}

pub async fn admin_delete_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    let identity = match check_admin_auth(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return *resp,
    };

    let removed = {
        let mut clients = state.clients.write().unwrap();
        let before = clients.len();
        clients.retain(|c| !(c.name == name && in_admin_scope(&identity, c)));
        clients.len() != before
    };
    if !removed {
        return json_error(404, &format!("no client named \"{name}\""));
    }

    state
        .client_keys
        .write()
        .unwrap()
        .retain(|_, (n, _)| n != &name);
    state.router.remove_client(&name);
    admin_audit_log(&identity, "delete_client", &name);

    Json(json!({ "status": "ok" })).into_response()
}

pub async fn chat_completions(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    let client_name = resolve_client_identity(&state, &headers).await;

    let mut rate_limit_status = None;
    if let Some((identity, rpm)) = resolve_rate_limit(&state, client_name.as_deref(), addr) {
        match state.rate_limiter.check(&identity, rpm) {
            Ok(status) => rate_limit_status = Some(status),
            Err(status) => return rate_limited_response(&state, &identity, &status),
        }
    }

    let mut resp = chat_completions_dispatch(&state, client_name, req).await;
    if let Some(status) = &rate_limit_status {
        apply_rate_limit_headers(&mut resp, status);
    }
    resp
}

/// `POST /v1/embeddings` -- same auth and inbound rate-limiting as
/// `chat_completions`, but dispatches straight to `Router::embeddings`
/// rather than the chat pipeline: no preset/guardrails/moderation/
/// web-search/budget stages apply here, since none of those have an
/// established meaning for an embeddings call yet.
pub async fn embeddings(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<EmbeddingsRequest>,
) -> Response {
    if let Some(resp) = check_auth(&state, &headers).await {
        return resp;
    }

    let client_name = resolve_client_identity(&state, &headers).await;

    let mut rate_limit_status = None;
    if let Some((identity, rpm)) = resolve_rate_limit(&state, client_name.as_deref(), addr) {
        match state.rate_limiter.check(&identity, rpm) {
            Ok(status) => rate_limit_status = Some(status),
            Err(status) => return rate_limited_response(&state, &identity, &status),
        }
    }

    let mut resp = match state.router.embeddings(&req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => router_error_response(e),
    };
    if let Some(status) = &rate_limit_status {
        apply_rate_limit_headers(&mut resp, status);
    }
    resp
}

/// The part of `chat_completions` downstream of auth/rate-limiting --
/// split out so that seam is the single place `chat_completions` attaches
/// `X-RateLimit-*` headers, regardless of which of these branches produced
/// the response.
async fn chat_completions_dispatch(
    state: &AppState,
    client_name: Option<String>,
    mut req: ChatRequest,
) -> Response {
    if req.messages.is_empty() {
        return json_error(400, "\"messages\" must not be empty");
    }

    if let Err(e) = state.router.apply_preset(&mut req) {
        return router_error_response(e);
    }

    state.router.apply_web_search(&mut req).await;

    if let Err(e) = state.router.apply_guardrails(&mut req) {
        return router_error_response(e);
    }

    if let Err(e) = state.router.apply_moderation(&req).await {
        return router_error_response(e);
    }

    if let Some(name) = &client_name {
        if let Err(exceeded) = state.router.check_client_budget(name).await {
            return budget_exceeded_response(state, name, exceeded);
        }
    }

    if req.is_streaming() {
        match state.router.dispatch_stream(&req).await {
            Ok(chunk_stream) => {
                let router = state.router.clone();
                let events = chunk_stream
                    .map(move |item| {
                        let event = match item {
                            Ok(chunk) => {
                                if let Some(name) = &client_name {
                                    if let Some(usage) = &chunk.usage {
                                        router.record_client_daily_usage(
                                            name,
                                            usage,
                                            chunk.cost_usd,
                                        );
                                    }
                                    if let Some(cost) = chunk.cost_usd {
                                        router.record_client_spend(name, cost);
                                    }
                                }
                                Event::default()
                                    .json_data(&chunk)
                                    .unwrap_or_else(|_| Event::default().data("{}"))
                            }
                            Err(e) => Event::default()
                                .event("error")
                                .data(json!({"error": {"message": e.to_string()}}).to_string()),
                        };
                        Ok::<_, Infallible>(event)
                    })
                    .chain(stream::once(async { Ok(Event::default().data("[DONE]")) }));

                Sse::new(events)
                    .keep_alive(KeepAlive::default())
                    .into_response()
            }
            Err(e) => router_error_response(e),
        }
    } else {
        match state.router.dispatch(&req).await {
            Ok(resp) => {
                if let Some(name) = &client_name {
                    if let Some(usage) = &resp.usage {
                        state
                            .router
                            .record_client_daily_usage(name, usage, resp.cost_usd);
                    }
                    if let Some(cost) = resp.cost_usd {
                        state.router.record_client_spend(name, cost);
                    }
                }
                Json(resp).into_response()
            }
            Err(e) => router_error_response(e),
        }
    }
}
