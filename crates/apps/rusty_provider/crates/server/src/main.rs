use std::sync::{Arc, RwLock};

use rp_core::RateLimiter;
use rp_router::{Config, Router as ProviderRouter};
use rp_server::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rusty_log::tracing_subscriber::fmt()
        .with_env_filter(
            rusty_log::tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rp_server=info".parse()?),
        )
        .init();

    let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
    let config = Config::from_file(&config_path)
        .map_err(|e| anyhow::anyhow!("{e}\n\nSee config.example.toml for a starting point."))?;

    let router = Arc::new(ProviderRouter::from_config(&config).await);
    let configured: Vec<&str> = router.configured_providers().collect();
    if configured.is_empty() {
        rusty_log::warn!("no providers configured (check that their api_key_env vars are set) — every request will fail");
    } else {
        rusty_log::info!(providers = ?configured, "providers ready");
    }

    // Design review 3.1: a configured credential that does not resolve is a
    // startup error, not a warning followed by less (or no) authentication.
    let credentials =
        rp_server::credentials::resolve_credentials(&config, &|var| std::env::var(var).ok())
            .map_err(|e| anyhow::anyhow!(e))?;
    let (api_key, client_keys, jwt) = (
        credentials.api_key,
        credentials.client_keys,
        credentials.jwt,
    );
    if api_key.is_none() && client_keys.is_empty() && jwt.is_none() {
        if config.server.allow_unauthenticated {
            rusty_log::warn!("no caller authentication configured and server.allow_unauthenticated is set: /v1 is open to anyone who can reach it");
        } else {
            rusty_log::warn!("no caller authentication configured: /v1 answers 401 until a client is added (or set server.allow_unauthenticated)");
        }
    }
    if !client_keys.is_empty() {
        rusty_log::info!(clients = ?config.clients.iter().map(|c| &c.name).collect::<Vec<_>>(), "named clients ready");
    }
    if jwt.is_some() {
        rusty_log::info!("JWT auth enabled");
    }

    let admin_key = config
        .server
        .admin_key_env
        .as_ref()
        .and_then(|var| std::env::var(var).ok());
    if admin_key.is_none() && config.server.admin_key_env.is_some() {
        rusty_log::warn!(
            "server.admin_key_env is set in config but the env var isn't — the admin API stays disabled"
        );
    } else if admin_key.is_some() {
        rusty_log::info!("admin API enabled");
    }

    let mcp = match &config.mcp {
        Some(mcp_config) if mcp_config.enabled => {
            let handler = rp_mcp::build(mcp_config, Arc::clone(&router)).await?;
            rusty_log::info!(path = %mcp_config.path, upstreams = mcp_config.upstreams.len(), "MCP endpoint enabled");
            Some(handler)
        }
        _ => None,
    };
    let mcp_path = config
        .mcp
        .as_ref()
        .map(|c| c.path.clone())
        .unwrap_or_else(|| "/mcp".to_string());

    let concurrency_limiter = config.server.max_concurrent_requests.map(|max| {
        rusty_log::info!(max, "global concurrency cap enabled");
        Arc::new(tokio::sync::Semaphore::new(max))
    });

    let state = AppState {
        router,
        api_key,
        client_keys: Arc::new(RwLock::new(client_keys)),
        default_rate_limit_rpm: config.server.default_rate_limit_rpm,
        rate_limiter: Arc::new(RateLimiter::new()),
        clients: Arc::new(RwLock::new(config.clients.clone())),
        admin_key,
        max_body_bytes: config.server.max_body_bytes,
        jwt,
        mcp,
        mcp_path,
        concurrency_limiter,
        cors_allowed_origins: config.server.cors_allowed_origins.clone(),
        allow_unauthenticated: config.server.allow_unauthenticated,
    };

    if std::env::var("MCP_STDIO").is_ok() {
        let Some(mcp) = state.mcp.clone() else {
            anyhow::bail!("MCP_STDIO is set but [mcp].enabled is not true in config");
        };
        rusty_log::info!("serving MCP over stdio (MCP_STDIO set)");
        // The server's handlers block, so serve from a thread of its own.
        tokio::task::spawn_blocking(move || rusty_mcp_server::serve_stdio(mcp)).await??;
        return Ok(());
    }

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let tokio = tokio::runtime::Handle::current();
    // The server runs on a rusty_tokio runtime of its own (#875); this
    // tokio runtime stays up for the handlers' tokio libraries.
    tokio::task::spawn_blocking(move || serve_on_rusty_tokio(state, tokio, &addr)).await?
}

/// Serves until the process is stopped, as `axum::serve` did.
fn serve_on_rusty_tokio(
    state: AppState,
    tokio: tokio::runtime::Handle,
    addr: &str,
) -> anyhow::Result<()> {
    let runtime = rusty_tokio::Builder::new_multi_thread()
        .thread_name("rp-server")
        .build()?;
    runtime.block_on(async {
        let listener = rusty_tokio::io::TcpListener::bind_addrs(addr).await?;
        let server = rp_server::serve(state, tokio, listener)?;
        rusty_log::info!(addr = %server.local_addr(), "rusty_provider listening");
        server.shutdown_on(std::future::pending()).await;
        Ok(())
    })
}
