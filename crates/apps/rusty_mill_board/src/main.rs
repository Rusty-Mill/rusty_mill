//! `rusty_mill_board`: the board as an MCP server (stdio or HTTP) with a web UI.
//!
//! ```text
//! rusty_mill_board --store board.mmap                    # MCP over stdio
//! rusty_mill_board --store board.mmap --http 127.0.0.1:8787 --auth-token S
//! ```
//! Add `--github-repo owner/name` (and `GITHUB_TOKEN`) to mirror cards to issues.
//! Add `--identities FILE` for a token per model, each bound to an actor and role.
//! The HTTP mode serves the UI at `/`, the web API at `/api/<op>` and MCP at
//! `/mcp`; open `http://127.0.0.1:8787/#token=S` for the UI.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rusty_mcp_server::{HttpConfig, HttpHandler, serve_stdio};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::github::{GithubConfig, GithubForge};
use rusty_mill_board::identities::Credentials;
use rusty_mill_board::mcp;
use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::sync::reconcile;
use rusty_mill_board::web::{Auth, Web, check_bind};

/// Mill Board: kanban for humans and agents.
#[derive(Parser)]
struct Cli {
    /// Board file (created if absent). Omit for an in-memory board.
    #[arg(long, env = "MILL_BOARD_STORE")]
    store: Option<PathBuf>,
    /// Serve HTTP (UI + API + MCP) on this address instead of MCP over stdio.
    #[arg(long, env = "MILL_BOARD_HTTP")]
    http: Option<SocketAddr>,
    /// One shared bearer token required on /api and /mcp (callers state their own actor and role).
    #[arg(long, env = "MILL_BOARD_TOKEN", hide_env_values = true)]
    auth_token: Option<String>,
    /// JSON file of per-model credentials: a token bound to an actor and role each
    /// (must be chmod 600). Callers then cannot choose their actor or role.
    #[arg(long, env = "MILL_BOARD_IDENTITIES", conflicts_with = "auth_token")]
    identities: Option<PathBuf>,
    /// Mirror the board to GitHub issues of this repo (`owner/name`). The token
    /// comes from `GITHUB_TOKEN` and needs issues read/write on the repo.
    #[arg(long, env = "MILL_BOARD_GITHUB_REPO")]
    github_repo: Option<String>,
    /// GitHub REST base URL (GitHub Enterprise Server: `https://host/api/v3`).
    #[arg(
        long,
        env = "MILL_BOARD_GITHUB_API",
        default_value = "https://api.github.com"
    )]
    github_api: String,
    /// GitHub token.
    #[arg(long, env = "GITHUB_TOKEN", hide_env_values = true)]
    github_token: Option<String>,
    /// Seconds between sync runs.
    #[arg(long, env = "MILL_BOARD_GITHUB_SECS", default_value_t = 60, value_parser = clap::value_parser!(u64).range(5..))]
    github_secs: u64,
    /// Serve with no authentication on a non-loopback address. Anyone who can reach the
    /// port can act as any role, including human. Almost never what you want.
    #[arg(long)]
    accept_unauthenticated_public: bool,
    /// Accepted Host names (HTTP only).
    #[arg(long = "allowed-host", value_delimiter = ',', default_values_t = ["127.0.0.1".to_owned(), "localhost".to_owned(), "[::1]".to_owned()])]
    allowed_hosts: Vec<String>,
}

/// Run [`reconcile`] on a background thread, forever. Failures are logged and
/// retried on the next tick; the board keeps serving regardless.
fn spawn_sync(board: Arc<Shared>, config: GithubConfig, every: Duration) -> Result<(), String> {
    let forge = GithubForge::new(config).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("github-sync".into())
        .spawn(move || {
            loop {
                match reconcile(&board, &forge) {
                    Ok(r) if r.is_quiet() && r.drift.is_empty() && r.missing.is_empty() => {}
                    Ok(r) => tracing::info!(?r, "github sync"),
                    Err(e) => tracing::warn!(error = %e, "github sync failed; will retry"),
                }
                std::thread::sleep(every);
            }
        })
        .map(drop)
        .map_err(|e| format!("cannot start sync thread: {e}"))
}

fn run(cli: Cli) -> Result<(), String> {
    let store: Box<dyn Store> = match &cli.store {
        Some(path) => Box::new(MmdbStore::open(path).map_err(|e| e.to_string())?),
        None => Box::new(MemStore::default()),
    };
    let board: Arc<Shared> = Arc::new(Board::new(store, Roles::default()));
    if let Some(repo) = &cli.github_repo {
        let token = cli
            .github_token
            .as_deref()
            .ok_or("--github-repo needs GITHUB_TOKEN")?;
        let config = GithubConfig::new(&cli.github_api, repo, token)?;
        spawn_sync(board.clone(), config, Duration::from_secs(cli.github_secs))?;
    }
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
    let auth = match (&cli.identities, cli.auth_token) {
        (Some(path), _) => Auth::Credentials(Credentials::load(path, &Roles::default())?),
        (None, Some(token)) => Auth::Shared(token),
        (None, None) => Auth::Open,
    };
    check_bind(addr, &auth, cli.accept_unauthenticated_public)?;
    let web = Web::new(
        board,
        HttpHandler::new(server, config),
        mcp_path,
        cli.allowed_hosts,
        auth,
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
