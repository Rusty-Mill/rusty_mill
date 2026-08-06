use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use rp_core::RateLimiter;
use rp_mcp::RustyMcpServer;
use rp_router::{ClientConfig, Router};
use tokio::sync::Semaphore;

use crate::jwt::JwtVerifier;

#[derive(Clone)]
pub struct AppState {
    /// Owns per-client spend budget tracking (`Router::check_client_budget`/
    /// `record_client_spend`) in addition to dispatch -- there's no
    /// separate client-budget type in this crate anymore, since sharing
    /// state with `[persistence]` requires living alongside it in
    /// `rp-router`.
    pub router: Arc<Router>,
    /// Bearer token clients must present to this router's own API, if
    /// `server.api_key_env` was set in config and the env var resolved.
    /// Any key in `client_keys` below also authenticates, independent of
    /// this field.
    pub api_key: Option<String>,
    /// Resolved API key string -> (client name, requests-per-minute).
    /// Presenting one of these keys both authenticates the request and
    /// buckets its rate limit under the client's name instead of the
    /// source-IP fallback. Lock-protected (unlike the rest of this
    /// struct's config-derived fields) since the admin API's runtime
    /// client provisioning endpoints add/update/remove entries here after
    /// startup.
    pub client_keys: Arc<RwLock<HashMap<String, (String, u32)>>>,
    /// Requests-per-minute limit for callers not matched to `client_keys`,
    /// bucketed by source IP. `None` means no limit for such callers.
    pub default_rate_limit_rpm: Option<u32>,
    pub rate_limiter: Arc<RateLimiter>,
    /// Every configured or runtime-provisioned `[[clients]]` entry, for the
    /// admin API (`GET /v1/admin/clients`) to enumerate -- `client_keys`
    /// above is keyed by API key (for authenticating inbound requests),
    /// not by name, so it can't be listed the other way around. Kept in
    /// sync with `client_keys` by every admin create/update/delete
    /// handler.
    pub clients: Arc<RwLock<Vec<ClientConfig>>>,
    /// Bearer token that unlocks `/v1/admin/*`, if `server.admin_key_env`
    /// was set in config and the env var resolved. `None` disables the
    /// admin API entirely, independent of `api_key`/`client_keys` above.
    pub admin_key: Option<String>,
    /// Ceiling on an inbound request body, in bytes -- `server.max_body_bytes`,
    /// applied as a `DefaultBodyLimit` layer over the whole router in
    /// `build_app`.
    pub max_body_bytes: usize,
    /// JWT/OIDC verifier, if `[jwt]` was configured and at least one of
    /// its modes (`hs256_secret_env` resolved, or `jwks_url` set)
    /// actually activated. `None` means a presented bearer token can only
    /// ever satisfy `api_key`/`client_keys` above, same as before this
    /// field existed. Only consulted by `check_auth`, never
    /// `check_admin_auth` -- `/v1/admin/*` stays `admin_key`/admin-role
    /// clients only.
    pub jwt: Option<Arc<JwtVerifier>>,
    /// The combined MCP handler (native tools + proxied upstreams), if
    /// `[mcp].enabled = true`. `None` means the MCP endpoint isn't mounted
    /// at all -- `build_app` skips it entirely rather than mounting a
    /// disabled stub.
    pub mcp: Option<Arc<RustyMcpServer>>,
    /// Path the MCP endpoint is mounted at when `mcp` is `Some`
    /// (`[mcp].path`, default `/mcp`). Unused otherwise.
    pub mcp_path: String,
    /// Server-wide in-flight request cap (`server.max_concurrent_requests`),
    /// enforced by `concurrency_limit` as a `try_acquire` -- a request that
    /// arrives once every permit is checked out gets `503` immediately
    /// rather than queuing. `None` means no cap, the same as before this
    /// field existed. Distinct from `rate_limiter` above: that bounds one
    /// caller's *rate*, this bounds the *total in-flight count* across
    /// every caller and route.
    pub concurrency_limiter: Option<Arc<Semaphore>>,
}
