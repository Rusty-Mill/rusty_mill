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
use rusty_mill_board::mcp;
use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::projects::Projects;
use rusty_mill_board::settings::{
    Files, GithubFile, Settings, load_github, load_roles, open_credentials,
};
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::syncer::Syncer;
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
    /// Directory holding the settings files (identities.json, projects.json, roles.json,
    /// github.json). The UI's settings view edits them in place; a missing identities.json is
    /// created with a first admin whose token is printed once.
    #[arg(long, env = "MILL_BOARD_SETTINGS_DIR", conflicts_with = "auth_token")]
    settings_dir: Option<PathBuf>,
    /// JSON file of per-model credentials: a token bound to an actor and role each
    /// (must be chmod 600). Callers then cannot choose their actor or role. Created with a
    /// first admin if it does not exist.
    #[arg(long, env = "MILL_BOARD_IDENTITIES", conflicts_with = "auth_token")]
    identities: Option<PathBuf>,
    /// JSON file of roles and the capabilities each holds: `{"builder": ["build"]}`. Without
    /// it, the default roles (human, builder, qa, reviewer).
    #[arg(long, env = "MILL_BOARD_ROLES")]
    roles: Option<PathBuf>,
    /// JSON file of the GitHub sync (repo, api, interval_secs, token; must be chmod 600).
    /// Takes the place of the --github-* flags, which only seed it.
    #[arg(long, env = "MILL_BOARD_GITHUB_CONFIG")]
    github_config: Option<PathBuf>,
    /// JSON file naming the projects (applications) this board serves and the repository
    /// paths each may change: `{"rocket_league": ["crates/apps/rocket_league"]}`. Without it
    /// there is only the `default` project.
    #[arg(long, env = "MILL_BOARD_PROJECTS")]
    projects: Option<PathBuf>,
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

/// The settings files: the directory's, each overridden by its own flag.
fn files(cli: &Cli) -> Files {
    let mut files = cli
        .settings_dir
        .as_deref()
        .map(Files::in_dir)
        .unwrap_or_default();
    files.identities = cli.identities.clone().or(files.identities);
    files.projects = cli.projects.clone().or(files.projects);
    files.roles = cli.roles.clone().or(files.roles);
    files.github = cli.github_config.clone().or(files.github);
    files
}

/// A file named by its own flag must exist; one found through `--settings-dir` may be missing.
fn required(flag: &Option<PathBuf>, name: &str) -> Result<(), String> {
    match flag {
        Some(path) if !path.exists() => Err(format!("{name}: {} does not exist", path.display())),
        _ => Ok(()),
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let store: Box<dyn Store> = match &cli.store {
        Some(path) => Box::new(MmdbStore::open(path).map_err(|e| e.to_string())?),
        None => Box::new(MemStore::default()),
    };
    let files = files(&cli);
    required(&cli.projects, "--projects")?;
    required(&cli.roles, "--roles")?;
    let roles = match &files.roles {
        Some(path) if path.exists() => load_roles(path)?,
        _ => Roles::default(),
    };
    let projects = match &files.projects {
        Some(path) if path.exists() => Projects::load(path)?,
        _ => Projects::default(),
    };
    let board: Arc<Shared> =
        Arc::new(Board::new(store, roles.clone()).with_projects(projects.clone()));

    // The GitHub sync starts from its file, else from the flags.
    let github = match files
        .github
        .as_deref()
        .map(load_github)
        .transpose()?
        .flatten()
    {
        Some(file) => Some(file),
        None => match &cli.github_repo {
            Some(repo) => Some(GithubFile {
                repo: repo.clone(),
                api: cli.github_api.clone(),
                interval_secs: cli.github_secs,
                token: cli
                    .github_token
                    .clone()
                    .ok_or("--github-repo needs GITHUB_TOKEN")?,
            }),
            None => None,
        },
    };
    let syncer = Syncer::new(board.clone());
    if let Some(file) = &github {
        let config = file.config()?;
        syncer.start(config, &file.repo, Duration::from_secs(file.interval_secs))?;
    }

    let server = mcp::server(board.clone()).map_err(|e| e.to_string())?;
    let Some(addr) = cli.http else {
        let _settings = Settings::new(board, files, None, syncer, github);
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
    let creds = match &files.identities {
        Some(path) => {
            let (creds, first) = open_credentials(path, &roles, &projects)?;
            if let Some(token) = first {
                eprintln!(
                    "rusty_mill_board: created {} with a first admin, named admin. Its token, shown once: {token}",
                    path.display()
                );
            }
            Some(creds.shared())
        }
        None => None,
    };
    let auth = match (&creds, cli.auth_token) {
        (Some(creds), _) => Auth::Credentials(Arc::clone(creds)),
        (None, Some(token)) => Auth::Shared(token),
        (None, None) => Auth::Open,
    };
    check_bind(addr, &auth, cli.accept_unauthenticated_public)?;
    let settings = Arc::new(Settings::new(board.clone(), files, creds, syncer, github));
    let web = Web::new(
        board,
        HttpHandler::new(server, config),
        mcp_path,
        cli.allowed_hosts,
        auth,
    )
    .with_settings(settings);
    let http = rusty_serve::Server::bind_shared(addr, web)
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    rusty_log::info!(%addr, "Mill Board listening");
    http.run().map_err(|e| format!("HTTP server failed: {e}"))
}

fn main() -> ExitCode {
    let _ = rusty_log::tracing_subscriber::fmt()
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
