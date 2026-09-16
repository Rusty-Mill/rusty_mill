//! `meshed-registry`: serves the data-product registry HTTP API -- the
//! Rust counterpart of the source repo's `uvicorn meshed.registry.app:app
//! --port 8100` command, which is how `data-mesh-monitor` and the CLI
//! reach the registry.
//!
//! Startup mirrors the source app's lifespan (REG-002/REG-003): open the
//! SQLite engine at `PlatformConfig::registry_db_path` (`MESHED_REGISTRY_
//! DB_PATH`, default `meshed_registry.db`) and create every table up
//! front, then serve. The bind address comes from `MESHED_REGISTRY_BIND`
//! (default `127.0.0.1:8100`, the port the dashboard's Vite proxy and the
//! source README both assume).

use rusty_meshed_registry::app::{build_router, create_all, get_config, AppState};
use rusty_meshed_registry::http::server::serve;
use rusty_tokio::io::TcpListener;
use std::sync::Arc;

const BIND_ENV: &str = "MESHED_REGISTRY_BIND";
const DEFAULT_BIND: &str = "127.0.0.1:8100";

#[rusty_tokio::main]
async fn main() {
    let config = match get_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("meshed-registry: invalid configuration: {err}");
            std::process::exit(2);
        }
    };

    let mut state = AppState::new();
    state.set_engine(config.registry_db_path.clone());
    match state.get_session() {
        Ok(conn) => {
            if let Err(err) = create_all(&conn) {
                eprintln!(
                    "meshed-registry: failed to create tables in {}: {err}",
                    config.registry_db_path
                );
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!(
                "meshed-registry: failed to open {}: {err}",
                config.registry_db_path
            );
            std::process::exit(1);
        }
    }

    let bind = std::env::var(BIND_ENV).unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let addr = match bind.parse() {
        Ok(addr) => addr,
        Err(err) => {
            eprintln!("meshed-registry: {BIND_ENV}={bind:?} is not a socket address: {err}");
            std::process::exit(2);
        }
    };
    let listener = match TcpListener::bind(addr) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("meshed-registry: failed to bind {bind}: {err}");
            std::process::exit(1);
        }
    };
    let local = listener.local_addr().map(|a| a.to_string()).unwrap_or(bind);
    eprintln!(
        "meshed-registry: serving on http://{local} (db: {})",
        config.registry_db_path
    );

    let router = Arc::new(build_router(Arc::new(state)));
    if let Err(err) = serve(listener, router).await {
        eprintln!("meshed-registry: server error: {err}");
        std::process::exit(1);
    }
}
