//! `rusty_mill_board`: the board as an MCP server (stdio or HTTP) with a web UI.
//!
//! ```text
//! RUSTY_TICK_TOKEN=T rusty_mill_board --tick-url http://127.0.0.1:8788   # MCP over stdio
//! RUSTY_TICK_TOKEN=T rusty_mill_board --tick-url http://127.0.0.1:8788 --http 127.0.0.1:8787 --auth-token S
//! ```
//! The cards are rusty_tick tasks tagged `agent` (ADR-0012); without `--tick-url`
//! the board lives in memory and is gone when the process ends.
//! Add `--github-repo owner/name` (and `GITHUB_TOKEN`) to mirror cards to issues.
//! Add `--identities FILE` for a token per model, each bound to an actor and role.
//! The HTTP mode serves the UI at `/`, the web API at `/api/<op>` and MCP at
//! `/mcp`; open `http://127.0.0.1:8787/#token=S` for the UI. `GET /health` answers
//! 200 while the store can commit and 503 once it needs a restart. `--issuer
//! https://board.example.com` adds OAuth sign-in for claude.ai connectors. SIGTERM and
//! SIGINT stop the server cleanly.

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rusty_mcp_server::{HttpConfig, HttpHandler, serve_stdio};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::drain::{InFlight, begin_stop, serve_until_drained};
use rusty_mill_board::mcp;
use rusty_mill_board::projects::Projects;
use rusty_mill_board::settings::{
    Files, GithubFile, Settings, load_github, load_roles, open_credentials,
};
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::syncer::Syncer;
use rusty_mill_board::tick::{TickHttp, TickStore};
use rusty_mill_board::web::{Auth, Web, check_bind, issuer_hosts};
use rusty_oauth_server::{Issuer, validate_issuer};

/// OAuth's grant file, by default.
const OAUTH_STATE: &str = "oauth.json";

/// How long a stop waits for running requests before it ends them.
const DRAIN: Duration = Duration::from_secs(10);

/// Settings of the removed board file and its backups (ADR-0012 step 6). A service configured
/// with them would otherwise start an empty in-memory board and lose its writes at restart.
const RETIRED_ENV: [&str; 4] = [
    "MILL_BOARD_STORE",
    "MILL_BOARD_BACKUP_DIR",
    "MILL_BOARD_BACKUP_SECS",
    "MILL_BOARD_BACKUP_KEEP",
];

/// Refuse to start while a retired setting is present, before anything is opened or started.
fn refuse_retired_env() -> Result<(), String> {
    let set: Vec<&str> = RETIRED_ENV
        .into_iter()
        .filter(|name| std::env::var_os(name).is_some())
        .collect();
    if set.is_empty() {
        return Ok(());
    }
    Err(format!(
        "retired setting(s) {}: the board file and its backups were removed and the cards live in \
         rusty_tick (ADR-0012). Unset them and set MILL_BOARD_TICK_URL and RUSTY_TICK_TOKEN",
        set.join(", ")
    ))
}

/// Mill Board: kanban for humans and agents.
#[derive(Parser)]
struct Cli {
    /// Export validated project ownership and exit before opening stores or services.
    /// Requires --projects; emits the existing projects file format.
    #[arg(long, requires = "projects")]
    export_projects: bool,
    /// Keep the cards in this rusty_tick (e.g. `http://127.0.0.1:8788`): its tasks tagged
    /// `agent` are the board. Omit for an in-memory board. The token is read from the
    /// `RUSTY_TICK_TOKEN` environment variable only, never an argument (which process
    /// listings show).
    #[arg(long, env = "MILL_BOARD_TICK_URL")]
    tick_url: Option<String>,
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
    /// Serve OAuth sign-in for /mcp under this public origin (an https URL, e.g.
    /// `https://board.example.com`), so a claude.ai connector can "Sign in now". The consent
    /// page takes one of the board's own tokens. Its host is accepted as a Host name.
    #[arg(long, env = "MILL_BOARD_ISSUER")]
    issuer: Option<String>,
    /// Where OAuth keeps its grants (owner-only JSON), bound to the issuer on first use.
    /// Default: `oauth.json` in --settings-dir, else `<identities stem>.oauth.json` beside
    /// --identities.
    #[arg(long, env = "MILL_BOARD_OAUTH_STATE", requires = "issuer")]
    oauth_state: Option<PathBuf>,
    /// Accepted Host names (HTTP only).
    #[arg(long = "allowed-host", env = "MILL_BOARD_ALLOWED_HOSTS", value_delimiter = ',', default_values_t = ["127.0.0.1".to_owned(), "localhost".to_owned(), "[::1]".to_owned()])]
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

/// The issuer and grant file, when OAuth is on.
fn oauth(cli: &Cli, files: &Files) -> Result<Option<(Issuer, PathBuf)>, String> {
    let Some(raw) = &cli.issuer else {
        return Ok(None);
    };
    let issuer = validate_issuer(raw).map_err(|e| format!("--issuer {raw:?}: {e}"))?;
    let state = cli
        .oauth_state
        .clone()
        .or_else(|| default_oauth_state(cli.settings_dir.as_deref(), files.identities.as_deref()))
        .ok_or("--issuer needs --oauth-state, --settings-dir or --identities to keep grants in")?;
    Ok(Some((issuer, state)))
}

/// Where a board keeps its grants by default: `oauth.json` in its settings
/// directory, else `<identities stem>.oauth.json` beside its identities file,
/// so two boards whose identities share a directory do not share grants.
fn default_oauth_state(settings_dir: Option<&Path>, identities: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = settings_dir {
        return Some(dir.join(OAUTH_STATE));
    }
    let identities = identities?;
    let stem = identities.file_stem()?.to_string_lossy();
    Some(identities.with_file_name(format!("{stem}.{OAUTH_STATE}")))
}

/// A file named by its own flag must exist; one found through `--settings-dir` may be missing.
fn required(flag: &Option<PathBuf>, name: &str) -> Result<(), String> {
    match flag {
        Some(path) if !path.exists() => Err(format!("{name}: {} does not exist", path.display())),
        _ => Ok(()),
    }
}

/// Tell the operator the first admin's credential. On a terminal it is printed once; anywhere
/// else (a service's journal keeps stderr) it goes to an owner-only file beside the
/// credentials, and only the path is printed.
fn announce_first_admin(creds: &Path, secret: &str) -> Result<(), String> {
    if std::io::stderr().is_terminal() {
        eprintln!(
            "rusty_mill_board: created {} with a first admin, named admin. Its token, shown once: {secret}",
            creds.display()
        );
        return Ok(());
    }
    let file = creds.with_file_name("first-admin.token");
    rusty_atomic_file::write_private(&file, format!("{secret}\n").as_bytes())
        .map_err(|e| format!("{}: {e}", file.display()))?;
    eprintln!(
        "rusty_mill_board: created {} with a first admin, named admin. Its token is in {}; read it, then delete the file",
        creds.display(),
        file.display()
    );
    Ok(())
}

/// Stop `server` on SIGTERM or SIGINT (Ctrl+C elsewhere), so a service stop or restart ends the
/// accept loop and the process exits, instead of dying mid-request.
fn stop_on_signal(
    server: rusty_serve::ShutdownHandle,
    in_flight: Arc<InFlight>,
) -> Result<(), String> {
    let rt = rusty_tokio::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("signal runtime: {e}"))?;
    #[cfg(unix)]
    let mut term = {
        let _enter = rt.enter();
        rusty_tokio::signal::unix::signal(rusty_tokio::signal::unix::SignalKind::terminate())
            .map_err(|e| format!("SIGTERM handler: {e}"))?
    };
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            rt.block_on(async {
                #[cfg(unix)]
                rusty_tokio::select! {
                    _ = rusty_tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
                #[cfg(not(unix))]
                let _ = rusty_tokio::signal::ctrl_c().await;
            });
            rusty_log::info!("signal received, shutting down");
            begin_stop(&server, &in_flight);
        })
        .map(drop)
        .map_err(|e| format!("signal thread: {e}"))
}

fn run(cli: Cli) -> Result<(), String> {
    refuse_retired_env()?;
    if cli.export_projects {
        let path = cli
            .projects
            .as_deref()
            .ok_or("--export-projects requires --projects")?;
        println!("{}", Projects::load(path)?.file_json()?);
        return Ok(());
    }
    let store: Box<dyn Store> = match &cli.tick_url {
        Some(url) => {
            let token = std::env::var("RUSTY_TICK_TOKEN")
                .map_err(|_| "--tick-url needs RUSTY_TICK_TOKEN in the environment".to_owned())?;
            Box::new(TickStore::new(TickHttp::new(url, &token)?))
        }
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

    let in_flight: Arc<InFlight> = Arc::default();
    let server = mcp::server_with(board.clone(), in_flight.clone()).map_err(|e| e.to_string())?;
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
    let oauth = oauth(&cli, &files)?;
    let mut allowed_hosts = cli.allowed_hosts.clone();
    if let Some((issuer, _)) = &oauth {
        allowed_hosts.extend(issuer_hosts(issuer));
    }
    let config = HttpConfig {
        allowed_hosts: allowed_hosts.clone(),
        ..HttpConfig::default()
    };
    let mcp_path = config.path.clone();
    let creds = match &files.identities {
        Some(path) => {
            let (creds, first) = open_credentials(path, &roles, &projects)?;
            if let Some(secret) = first {
                announce_first_admin(path, &secret)?;
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
    let mut web = Web::new(
        board.clone(),
        HttpHandler::new(server, config),
        mcp_path,
        allowed_hosts,
        auth,
    )
    .with_settings(settings.clone())
    .with_in_flight(in_flight.clone());
    if let Some((issuer, state)) = oauth {
        rusty_log::info!(%issuer, state = %state.display(), "OAuth sign-in on");
        web = web.with_oauth(issuer, state)?;
    }
    let http = rusty_serve::Server::bind_shared(addr, web)
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    stop_on_signal(
        http.shutdown_handle().map_err(|e| e.to_string())?,
        in_flight.clone(),
    )?;
    rusty_log::info!(%addr, "Mill Board listening");
    // The accept loop ends on a signal. Then the requests being answered and the tool calls
    // running get one bounded wait: a commit whose reply is cut off is durable but
    // looks failed to its caller, who may then repeat it.
    match serve_until_drained(http, &in_flight, DRAIN) {
        Ok(true) => Ok(()),
        Ok(false) => {
            rusty_log::warn!(
                secs = DRAIN.as_secs(),
                "work still running at the deadline; stopping it"
            );
            Ok(())
        }
        Err(e) => Err(format!("HTTP server failed: {e}")),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_board_gets_its_own_default_oauth_state() {
        let a = default_oauth_state(None, Some(Path::new("/cfg/a.json")));
        let b = default_oauth_state(None, Some(Path::new("/cfg/b.json")));
        assert_eq!(a.as_deref(), Some(Path::new("/cfg/a.oauth.json")));
        assert_eq!(b.as_deref(), Some(Path::new("/cfg/b.oauth.json")));
        assert_eq!(
            default_oauth_state(Some(Path::new("/s")), Some(Path::new("/cfg/a.json"))).as_deref(),
            Some(Path::new("/s/oauth.json"))
        );
        assert_eq!(default_oauth_state(None, None), None);
    }
}
