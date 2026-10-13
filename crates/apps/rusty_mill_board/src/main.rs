//! `rusty_mill_board`: the board as an MCP server (stdio or HTTP) with a web UI.
//!
//! ```text
//! rusty_mill_board --store board.mmap                    # MCP over stdio
//! rusty_mill_board --store board.mmap --http 127.0.0.1:8787 --auth-token S
//! ```
//! The HTTP mode serves the UI at `/`, the web API at `/api/<op>` and MCP at
//! `/mcp`; open `http://127.0.0.1:8787/#token=S` for the UI.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use rusty_mcp_server::{HttpConfig, HttpHandler, serve_stdio};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::{mcp, web::Web};

/// Mill Board: kanban for humans and agents.
#[derive(Parser)]
struct Cli {
    /// Board file (created if absent). Omit for an in-memory board.
    #[arg(long, env = "MILL_BOARD_STORE")]
    store: Option<PathBuf>,
    /// Serve HTTP (UI + API + MCP) on this address instead of MCP over stdio.
    #[arg(long, env = "MILL_BOARD_HTTP")]
    http: Option<SocketAddr>,
    /// Bearer token required on /api and /mcp.
    #[arg(long, env = "MILL_BOARD_TOKEN")]
    auth_token: Option<String>,
    /// Accepted Host names (HTTP only).
    #[arg(long = "allowed-host", value_delimiter = ',', default_values_t = ["127.0.0.1".to_owned(), "localhost".to_owned(), "[::1]".to_owned()])]
    allowed_hosts: Vec<String>,
}

fn run(cli: Cli) -> Result<(), String> {
    let store: Box<dyn Store> = match &cli.store {
        Some(path) => Box::new(MmdbStore::open(path).map_err(|e| e.to_string())?),
        None => Box::new(MemStore::default()),
    };
    let board: Arc<Shared> = Arc::new(Board::new(store, Roles::default()));
    let server = mcp::server(board.clone()).map_err(|e| e.to_string())?;
    let Some(addr) = cli.http else {
        return serve_stdio(server).map_err(|e| format!("stdio failed: {e}"));
    };
    if cli
        .auth_token
        .as_deref()
        .is_some_and(|t| t.trim().is_empty())
    {
        return Err("--auth-token is empty".into());
    }
    let config = HttpConfig {
        allowed_hosts: cli.allowed_hosts.clone(),
        ..HttpConfig::default()
    };
    let mcp_path = config.path.clone();
    let web = Web::new(
        board,
        HttpHandler::new(server, config),
        mcp_path,
        cli.allowed_hosts,
        cli.auth_token,
    );
    let http = rusty_serve::Server::bind_shared(addr, web)
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    tracing::info!(%addr, "Mill Board listening");
    http.run().map_err(|e| format!("HTTP server failed: {e}"))
}

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("rusty_mill_board: {why}");
            ExitCode::FAILURE
        }
    }
}
