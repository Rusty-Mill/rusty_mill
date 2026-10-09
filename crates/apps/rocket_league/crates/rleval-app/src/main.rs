//! `rleval` — the unified RLEvalSystem application.
//!
//! One binary that replaces juggling the separate `replay-analyzer`,
//! `replay-scoring`, `replay-skills`, `replay-value` and `replay-viewer` CLIs:
//! it serves a web UI where you drop a `.replay` and get the 3D viewer, the
//! scoring report, the skills table and the value-impact analysis side by side,
//! all computed in-process by the [`pipeline`].
//!
//! ```text
//! rleval serve [--host <h>] [--port <p>] [--replays <dir>]
//! rleval analyze <file.replay> [--out <bundle.html>]   # one-shot, no server
//! ```

use std::error::Error;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rleval_app::auth::Accounts;
use rleval_app::authn::{
    clearing_cookie, session_cookie, Authn, AuthnError, Credentials, LoginProvider,
};
use rleval_app::history::{self, SessionRecord};
use rleval_app::jobs::{JobState, Jobs};
use rleval_app::panels::{PanelCache, Panels};
use rleval_app::progress;
use rleval_app::server::{self, Request, Response};
use rleval_app::store::{
    copy_all, session_key, AccountId, FsSessionStore, SaveOutcome, SessionStore,
};
use rleval_app::teams::{team_report, Team, Teams};
use rleval_app::uploads::{Signer, Upload, TTL_S};
use rleval_app::{admin, pipeline, ui};

const USAGE: &str = "\
usage:
  rleval serve [--host <host>] [--port <port>] [--replays <dir>] [--corpus <dir>] [--enable-admin-run]
               [--data-dir <dir>] [--store fs|mmdb] [--accounts <file>] [--teams <file>]
               [--oidc-client-id <id> --oidc-redirect-uri <url> --oidc-users <file>]
  rleval analyze <file.replay> [--out <bundle.html>]
  rleval import-json --data-dir <dir>      (needs a build with --features mmdb)

  serve    start the web UI (default http://127.0.0.1:8080; /admin shows model config)
           --enable-admin-run lets /admin trigger retrain/recalibrate (localhost only)
           --data-dir saves every analysis to per-account history and enables the History
             tab (win-vs-loss habits across matches); without it nothing is stored
           --store picks the backend under --data-dir: `fs` (default; one JSON file per
             session) or `mmdb` (Rusty-Mill engine; needs a build with --features mmdb)
           --teams <file> adds team workspaces (needs --accounts and --data-dir): one
             `team: coach:acct, player:acct=In-Game Name` per line; upload with a team to
             share a match; coaches see the whole roster, players only themselves
           --oidc-* turns on Google sign-in (needs a build with --features oidc): the
             browser signs in with Google and gets a session cookie. --oidc-users lists
             `email:account` — only those verified emails may sign in. The client secret
             is read from RLEVAL_OIDC_CLIENT_SECRET (never a flag). Overrides for another
             provider or a local mock: --oidc-auth-url/-token-url/-jwks-url/-issuer
           --accounts <file> requires `Authorization: Bearer <token>`; one `account:token`
             per line (tokens >= 16 chars). Omit for single-user mode (one `local` account)
  analyze  run the pipeline on one replay and write a self-contained HTML bundle
  import-json  copy sessions saved by the `fs` store into the `mmdb` store (idempotent)";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") | None => serve(args.collect()),
        Some("analyze") => analyze_oneshot(args.collect()),
        Some("import-json") => import_json(args.collect()),
        Some("-h") | Some("--help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other}\n{USAGE}").into()),
    }
}

// ---- serve ----

fn serve(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 8080;
    let mut replays = PathBuf::from("assets/replays");
    let mut corpus = PathBuf::from("assets/corpus");
    let mut allow_run = false;
    let mut data_dir: Option<PathBuf> = None;
    let mut store_kind: Option<StoreKind> = None;
    let mut oidc_flags = OidcFlags::default();
    let mut accounts_file: Option<PathBuf> = None;
    let mut teams_file: Option<PathBuf> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--host" => host = it.next().ok_or("--host needs a value")?,
            "--port" => port = it.next().ok_or("--port needs a value")?.parse()?,
            "--replays" => replays = PathBuf::from(it.next().ok_or("--replays needs a dir")?),
            "--corpus" => corpus = PathBuf::from(it.next().ok_or("--corpus needs a dir")?),
            "--enable-admin-run" => allow_run = true,
            "--data-dir" => {
                data_dir = Some(PathBuf::from(it.next().ok_or("--data-dir needs a dir")?))
            }
            "--store" => store_kind = Some(it.next().ok_or("--store needs fs|mmdb")?.parse()?),
            "--oidc-client-id" => oidc_flags.client_id = Some(flag_value(&mut it, &a)?),
            "--oidc-redirect-uri" => oidc_flags.redirect_uri = Some(flag_value(&mut it, &a)?),
            "--oidc-users" => oidc_flags.users = Some(PathBuf::from(flag_value(&mut it, &a)?)),
            "--oidc-auth-url" => oidc_flags.auth_url = Some(flag_value(&mut it, &a)?),
            "--oidc-token-url" => oidc_flags.token_url = Some(flag_value(&mut it, &a)?),
            "--oidc-jwks-url" => oidc_flags.jwks_url = Some(flag_value(&mut it, &a)?),
            "--oidc-issuer" => oidc_flags.issuer = Some(flag_value(&mut it, &a)?),
            "--teams" => teams_file = Some(PathBuf::from(it.next().ok_or("--teams needs a file")?)),
            "--accounts" => {
                accounts_file = Some(PathBuf::from(it.next().ok_or("--accounts needs a file")?))
            }
            other => return Err(format!("unknown flag {other}\n{USAGE}").into()),
        }
    }
    let accounts = match &accounts_file {
        Some(path) => Accounts::from_file(path)?,
        None => Accounts::open(),
    };
    let login = build_login(&oidc_flags)?;
    let authn = Authn::new(accounts, login);
    if authn.is_open() && host != "127.0.0.1" && host != "localhost" {
        eprintln!(
            "warning: no --accounts file or sign-in and host {host} is not loopback — anyone who can \
             reach this port shares the single `local` account"
        );
    }
    let teams = match &teams_file {
        Some(path) => {
            if authn.is_open() || data_dir.is_none() {
                return Err(
                    "--teams needs both --accounts or sign-in (roles need identity) and --data-dir"
                        .into(),
                );
            }
            let teams = Teams::from_file(path)?;
            teams.check_accounts(|a| authn.has_account(a))?;
            teams
        }
        None => Teams::none(),
    };
    let (store, team_store) = match (&data_dir, store_kind) {
        (Some(dir), kind) => {
            let stores = open_stores(kind.unwrap_or(StoreKind::Fs), dir)?;
            (Some(stores.people), Some(stores.pools))
        }
        (None, Some(_)) => return Err("--store needs --data-dir".into()),
        (None, None) => (None, None),
    };
    let state = Arc::new(AppState {
        replays,
        corpus,
        allow_run,
        store,
        team_store,
        authn,
        teams,
        panels: PanelCache::default(),
        jobs: Jobs::default(),
        signer: Signer::new().ok(),
    });

    eprintln!(
        "RLEval unified app serving on http://{host}:{port}  (sample dir: {})",
        state.replays.display()
    );
    match (&state.store, state.authn.is_open()) {
        (None, _) => eprintln!("history: off (pass --data-dir <dir> to save analyses per account)"),
        (Some(_), true) => eprintln!("history: on, single-user mode (account `local`)"),
        (Some(_), false) => eprintln!("history: on, authenticated (tokens and/or sign-in)"),
    }
    eprintln!("Open the URL in a browser, then drop a .replay file. Ctrl-C to stop.");
    if state.allow_run {
        eprintln!("admin maintenance endpoint ENABLED (/api/admin/run) — localhost only.");
    }
    server::serve(&host, port, move |req| route(req, &state))?;
    Ok(())
}

fn flag_value(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{flag} needs a value"))
}

/// Google (or other OpenID Connect) sign-in settings from the command line. The
/// client secret is deliberately not a flag — command lines are visible to other
/// users and end up in shell history — it comes from `RLEVAL_OIDC_CLIENT_SECRET`.
#[derive(Default)]
struct OidcFlags {
    client_id: Option<String>,
    redirect_uri: Option<String>,
    users: Option<PathBuf>,
    /// Overrides for a provider other than Google (or a local mock).
    auth_url: Option<String>,
    token_url: Option<String>,
    jwks_url: Option<String>,
    issuer: Option<String>,
}

impl OidcFlags {
    fn any(&self) -> bool {
        self.client_id.is_some()
            || self.redirect_uri.is_some()
            || self.users.is_some()
            || self.auth_url.is_some()
            || self.token_url.is_some()
            || self.jwks_url.is_some()
            || self.issuer.is_some()
    }
}

#[cfg(feature = "oidc")]
fn build_login(f: &OidcFlags) -> Result<Option<Box<dyn LoginProvider>>, Box<dyn Error>> {
    use rleval_app::oidc::{Oidc, OidcConfig, Users};
    use rleval_app::oidc_transport::StdTransport;

    if !f.any() {
        return Ok(None);
    }
    let client_id = f
        .client_id
        .clone()
        .ok_or("sign-in needs --oidc-client-id")?;
    let redirect = f
        .redirect_uri
        .clone()
        .ok_or("sign-in needs --oidc-redirect-uri")?;
    let users_file = f
        .users
        .as_ref()
        .ok_or("sign-in needs --oidc-users <file>")?;
    let secret = std::env::var("RLEVAL_OIDC_CLIENT_SECRET")
        .map_err(|_| "sign-in needs the client secret in RLEVAL_OIDC_CLIENT_SECRET")?;

    let mut config = OidcConfig::google(client_id, secret, redirect);
    if let Some(v) = &f.auth_url {
        config.authorization_url = v.clone();
    }
    if let Some(v) = &f.token_url {
        config.token_url = v.clone();
    }
    if let Some(v) = &f.jwks_url {
        config.jwks_url = v.clone();
    }
    if let Some(v) = &f.issuer {
        config.issuers = vec![v.clone()];
    }
    let users = Users::from_file(users_file)?;
    Ok(Some(Box::new(Oidc::new(
        config,
        users,
        Box::new(StdTransport),
    )?)))
}

#[cfg(not(feature = "oidc"))]
fn build_login(f: &OidcFlags) -> Result<Option<Box<dyn LoginProvider>>, Box<dyn Error>> {
    if f.any() {
        return Err("--oidc-* needs a build with `--features oidc`".into());
    }
    Ok(None)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StoreKind {
    Fs,
    Mmdb,
}

impl std::str::FromStr for StoreKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "fs" => Ok(Self::Fs),
            "mmdb" => Ok(Self::Mmdb),
            other => Err(format!("unknown --store {other:?} (use fs or mmdb)")),
        }
    }
}

/// The two stores a server keeps under one `--data-dir`.
struct Stores {
    /// Per-account histories, at `<dir>/<account>/`.
    people: Box<dyn SessionStore>,
    /// Shared team pools, at `<dir>/_teams/<team>/`.
    pools: Box<dyn SessionStore>,
}

fn open_stores(kind: StoreKind, dir: &Path) -> Result<Stores, Box<dyn Error>> {
    let pools = dir.join(TEAM_POOL_DIR);
    match kind {
        StoreKind::Fs => Ok(Stores {
            people: Box::new(FsSessionStore::new(dir)),
            pools: Box::new(FsSessionStore::new(pools)),
        }),
        #[cfg(feature = "mmdb")]
        StoreKind::Mmdb => Ok(Stores {
            people: Box::new(rleval_app::store_mmdb::MmdbSessionStore::new(dir)),
            pools: Box::new(rleval_app::store_mmdb::MmdbSessionStore::new(pools)),
        }),
        #[cfg(not(feature = "mmdb"))]
        StoreKind::Mmdb => Err("--store mmdb needs a build with `--features mmdb`".into()),
    }
}

/// `rleval import-json --data-dir <dir>`: copy sessions written by the `fs` store
/// (account histories and team pools) into the `mmdb` store in the same directory.
fn import_json(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let mut data_dir: Option<PathBuf> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data-dir" => {
                data_dir = Some(PathBuf::from(it.next().ok_or("--data-dir needs a dir")?))
            }
            other => return Err(format!("unknown flag {other}\n{USAGE}").into()),
        }
    }
    let dir = data_dir.ok_or("import-json needs --data-dir <dir>")?;
    let from = open_stores(StoreKind::Fs, &dir)?;
    let to = open_stores(StoreKind::Mmdb, &dir)?;
    let json = FsSessionStore::new(&dir);
    let pool_json = FsSessionStore::new(dir.join(TEAM_POOL_DIR));
    let mut total = rleval_app::store::CopyReport::default();
    for (label, from, to, namespaces) in [
        ("accounts", &from.people, &to.people, json.namespaces()?),
        (
            "team pools",
            &from.pools,
            &to.pools,
            pool_json.namespaces()?,
        ),
    ] {
        let r = copy_all(from.as_ref(), to.as_ref(), &namespaces)?;
        eprintln!(
            "{label}: {} copied, {} already present ({} namespaces)",
            r.copied,
            r.already_present,
            namespaces.len()
        );
        total.copied += r.copied;
        total.already_present += r.already_present;
    }
    eprintln!(
        "done: {} sessions imported. The JSON files were left in place; serve with --store mmdb.",
        total.copied
    );
    Ok(())
}

/// Everything the request handler needs, built once in [`serve`].
struct AppState {
    replays: PathBuf,
    corpus: PathBuf,
    allow_run: bool,
    /// `None` ⇒ history disabled (`--data-dir` not given).
    store: Option<Box<dyn SessionStore>>,
    /// Shared per-team match pools, under `<data-dir>/_teams` (a name no account
    /// can take — see `AccountId`). `Some` exactly when `store` is.
    team_store: Option<Box<dyn SessionStore>>,
    authn: Authn,
    teams: Teams,
    /// The viewer/scoring/ballchasing HTML of recent analyses, fetched lazily by the UI.
    panels: PanelCache,
    /// Analyses submitted through `POST /api/jobs`.
    jobs: Jobs,
    /// Signs upload URLs; `None` when the OS gave no randomness (uploads are then disabled).
    signer: Option<Signer>,
}

const TEAM_POOL_DIR: &str = "_teams";

fn route(req: &Request, state: &Arc<AppState>) -> Response {
    match (req.method.as_str(), req.route()) {
        ("GET", "/") => Response::html(ui::INDEX_HTML),
        ("GET", "/admin") => Response::html(ui::ADMIN_HTML),
        ("GET", "/healthz") => Response::text(200, "ok"),
        ("GET", "/api/auth") => auth_info(state),
        ("GET", "/api/me") => with_account(req, state, |account| {
            json_response(&serde_json::json!({ "account": account.as_str() }))
        }),
        ("GET", "/auth/login") => match state.authn.login() {
            None => Response::text(404, "sign-in is not configured"),
            Some(login) => match login.begin() {
                Ok(url) => Response::redirect(url),
                Err(e) => Response::text(500, e.to_string()),
            },
        },
        ("GET", "/auth/callback") => login_callback(req, state),
        ("POST", "/auth/logout") => logout(req, state),
        ("GET", "/api/samples") => samples_response(&state.replays),
        ("GET", "/api/config") => {
            json_response(&admin::config_report(&state.corpus, state.allow_run))
        }
        ("POST", "/api/admin/run") => {
            if !state.allow_run {
                return Response::text(403, "admin run disabled — start with --enable-admin-run");
            }
            let action = query_param(&req.path, "action").unwrap_or_default();
            match admin::run_action(&action, &state.corpus) {
                Some(r) => json_response(&r),
                None => Response::text(400, "unknown action"),
            }
        }
        ("POST", "/api/jobs") => with_account(req, state, |account| {
            match upload_team(req, state, account) {
                Ok(team) => submit_job(req, state, account, team.cloned()),
                Err(resp) => resp,
            }
        }),
        ("POST", "/api/uploads") => {
            with_account(req, state, |account| issue_upload(req, state, account))
        }
        // No account header: the signed URL itself is the credential.
        ("PUT", path) if path.starts_with("/api/uploads/") => {
            redeem_upload(req, state, path.trim_start_matches("/api/uploads/"))
        }
        ("GET", path) if path.starts_with("/api/jobs/") && path.ends_with("/events") => {
            with_account(req, state, |account| {
                let id = path
                    .trim_start_matches("/api/jobs/")
                    .trim_end_matches("/events");
                job_events(state, account, id)
            })
        }
        ("GET", path) if path.starts_with("/api/jobs/") => with_account(req, state, |account| {
            job_response(req, state, account, path.trim_start_matches("/api/jobs/"))
        }),
        ("POST", "/api/analyze") => with_account(req, state, |account| {
            let name = query_param(&req.path, "name").unwrap_or_else(|| "upload".to_string());
            let team = match upload_team(req, state, account) {
                Ok(t) => t,
                Err(resp) => return resp,
            };
            analyze_response(
                &req.body,
                &stem(&name),
                &AnalyzeOpts::from_request(req),
                state,
                account,
                team,
            )
        }),
        ("GET", path) if path.starts_with("/api/analyze/sample/") => {
            with_account(req, state, |account| {
                let name = percent_decode(path.trim_start_matches("/api/analyze/sample/"));
                // Guard against path traversal — sample names are flat filenames.
                if name.contains('/') || name.contains("..") {
                    return Response::text(400, "invalid sample name");
                }
                let team = match upload_team(req, state, account) {
                    Ok(t) => t,
                    Err(resp) => return resp,
                };
                match std::fs::read(state.replays.join(&name)) {
                    Ok(bytes) => analyze_response(
                        &bytes,
                        &stem(&name),
                        &AnalyzeOpts::from_request(req),
                        state,
                        account,
                        team,
                    ),
                    Err(e) => Response::text(404, format!("sample not found: {e}")),
                }
            })
        }
        ("GET", path) if path.starts_with("/api/analysis/") => {
            with_account(req, state, |account| {
                let (id, panel) = path
                    .trim_start_matches("/api/analysis/")
                    .split_once('/')
                    .unwrap_or_default();
                match state
                    .panels
                    .get(account, id)
                    .as_deref()
                    .and_then(|p| p.get(panel))
                {
                    Some(html) => Response::html(html),
                    None => Response::text(
                        404,
                        "no such panel (analyses are kept only briefly; re-run it)",
                    ),
                }
            })
        }
        ("GET", "/api/teams") => with_account(req, state, |account| {
            let mine: Vec<TeamEntry> = state
                .teams
                .for_account(account)
                .into_iter()
                .map(|(t, role)| TeamEntry {
                    team: t.id.to_string(),
                    role,
                    members: t.members.len(),
                })
                .collect();
            json_response(&mine)
        }),
        ("GET", path) if path.starts_with("/api/teams/") => with_account(req, state, |account| {
            let id = percent_decode(path.trim_start_matches("/api/teams/"));
            team_response(state, account, &id)
        }),
        ("GET", "/api/history") => with_history(req, state, |records| {
            json_response(&history::summarize(&records))
        }),
        ("GET", "/api/history/progress") => with_history(req, state, |records| {
            let Some(player) = query_param(&req.path, "player") else {
                return Response::text(400, "missing ?player=");
            };
            match progress::progress(&player, &records) {
                Some(report) => json_response(&report),
                None => Response::text(404, format!("no stored matches for {player:?}")),
            }
        }),
        ("GET", "/api/history/habits") => with_history(req, state, |records| {
            let Some(player) = query_param(&req.path, "player") else {
                return Response::text(400, "missing ?player=");
            };
            match history::habits(&player, &records) {
                Some(report) => json_response(&report),
                None => Response::text(404, format!("no stored matches for {player:?}")),
            }
        }),
        _ => Response::not_found(),
    }
}

/// Public: what the page needs to decide between "sign in" and the app.
fn auth_info(state: &AppState) -> Response {
    json_response(&serde_json::json!({
        "sign_in": state.authn.login().is_some(),
        "open": state.authn.is_open(),
    }))
}

/// The provider redirects the browser here: finish the login, set the session
/// cookie, and send the user to the app.
fn login_callback(req: &Request, state: &AppState) -> Response {
    let Some(login) = state.authn.login() else {
        return Response::text(404, "sign-in is not configured");
    };
    let query = req.path.split_once('?').map_or("", |(_, q)| q);
    match login.complete(query) {
        Ok(done) => Response::redirect("/").with_header(
            "Set-Cookie",
            session_cookie(&done.token, done.max_age_secs, login.secure_cookies()),
        ),
        // The message is the provider-agnostic, user-safe kind (see `LoginError`).
        Err(e) => Response::text(403, format!("sign-in failed: {e}")),
    }
}

fn logout(req: &Request, state: &AppState) -> Response {
    let Some(login) = state.authn.login() else {
        return Response::text(404, "sign-in is not configured");
    };
    // Same-origin rule as any cookie-authenticated write.
    if !state.authn.is_open() {
        let credentials = Credentials {
            method: &req.method,
            authorization: None,
            cookie: req.header("cookie"),
            origin: req.header("origin"),
            host: req.header("host"),
        };
        if let Err(AuthnError::CrossOrigin) = state.authn.authenticate(&credentials) {
            return Response::text(403, "cross-origin request refused");
        }
    }
    if let Some(token) = state.authn.session_token(req.header("cookie")) {
        login.end_session(token);
    }
    Response::text(200, "signed out")
        .with_header("Set-Cookie", clearing_cookie(login.secure_cookies()))
}

#[derive(serde::Serialize)]
struct TeamEntry {
    team: String,
    role: rleval_app::teams::Role,
    members: usize,
}

fn team_response(state: &AppState, account: &AccountId, id: &str) -> Response {
    let (Some(team), Some(pool)) = (state.teams.get(id), &state.team_store) else {
        return Response::text(404, "no such team");
    };
    // Same answer for "no such team" and "not on it": don't reveal team names.
    if team.member(account).is_none() {
        return Response::text(404, "no such team");
    }
    let records = match pool.list(team.id.namespace()) {
        Ok(r) => r,
        Err(e) => return Response::text(500, e.to_string()),
    };
    match team_report(team, account, &records) {
        Some(report) => json_response(&report),
        None => Response::text(404, "no such team"),
    }
}

/// Resolve `?team=` for an upload: `Ok(None)` when absent, the team when the
/// caller is on it, otherwise a ready-made error response (404 for an unknown
/// team *or* a non-member, matching [`team_response`]).
fn upload_team<'a>(
    req: &Request,
    state: &'a AppState,
    account: &AccountId,
) -> Result<Option<&'a Team>, Response> {
    let Some(id) = query_param(&req.path, "team") else {
        return Ok(None);
    };
    state
        .teams
        .get(&id)
        .filter(|t| t.member(account).is_some())
        .map(Some)
        .ok_or_else(|| Response::text(404, "no such team"))
}

fn json_response<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(json) => Response::json(json),
        Err(e) => Response::text(500, format!("serialize error: {e}")),
    }
}

/// Authenticate the request, then run `f` for that account; 401 on failure.
fn with_account(
    req: &Request,
    state: &AppState,
    f: impl FnOnce(&AccountId) -> Response,
) -> Response {
    let credentials = Credentials {
        method: &req.method,
        authorization: req.header("authorization"),
        cookie: req.header("cookie"),
        origin: req.header("origin"),
        host: req.header("host"),
    };
    match state.authn.authenticate(&credentials) {
        Ok(account) => f(&account),
        Err(e @ AuthnError::CrossOrigin) => Response::text(403, e.to_string()),
        Err(e) => Response::text(401, e.to_string()),
    }
}

/// What the caller asked of an analysis, from the query string.
struct AnalyzeOpts {
    /// `?rank=diamond`: pin the rank bracket instead of inferring it from the lobby.
    rank: Option<String>,
    /// `?inline=1`: the legacy response shape, with the HTML panels embedded.
    inline: bool,
    /// `?session=name`: the play session this match belongs to in history.
    session: Option<String>,
}

impl AnalyzeOpts {
    fn from_request(req: &Request) -> Self {
        Self {
            rank: query_param(&req.path, "rank"),
            inline: query_param(&req.path, "inline").is_some_and(|v| v == "1"),
            session: query_param(&req.path, "session"),
        }
    }
}

/// Like [`with_account`], for endpoints that read the account's stored sessions.
fn with_history(
    req: &Request,
    state: &AppState,
    f: impl FnOnce(Vec<SessionRecord>) -> Response,
) -> Response {
    let Some(store) = &state.store else {
        return Response::text(
            404,
            "history is disabled (start the server with --data-dir)",
        );
    };
    with_account(req, state, |account| match store.list(account) {
        Ok(records) => f(records),
        Err(e) => Response::text(500, e.to_string()),
    })
}

/// Run the pipeline and return the JSON bundle, isolating decoder panics. Loads
/// the rank-relative norms from the corpus dir (if present) so scores are graded
/// against their bracket; `opts.rank` pins an explicit rank when known. On success the
/// session is saved to `account`'s history when history is on, and (unless
/// `opts.inline`) the HTML panels are held for `/api/analysis/<id>/<panel>`.
fn analyze_json(
    bytes: &[u8],
    replay_id: &str,
    opts: &AnalyzeOpts,
    state: &AppState,
    account: &AccountId,
    team: Option<&Team>,
) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Err("empty request body — no replay bytes".into());
    }
    let norms = pipeline::load_rank_norms(&state.corpus);
    let xg = pipeline::load_xg(&state.corpus);
    let result = catch_unwind(AssertUnwindSafe(|| {
        pipeline::analyze(bytes, replay_id, norms.as_ref(), opts.rank.as_deref(), &xg)
    }));
    let mut analysis = match result {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => return Err(format!("could not analyze replay: {e}")),
        Err(_) => {
            return Err("could not decode replay (the file may be corrupt or unsupported)".into())
        }
    };
    persist(
        state,
        account,
        team,
        bytes,
        opts.session.as_deref(),
        &analysis,
    );
    if !opts.inline {
        // Default: just the data; the panels are fetched on first use.
        let id = session_key(bytes);
        state.panels.put(account, &id, Panels::take(&mut analysis));
        analysis.analysis_id = id;
    }
    serde_json::to_vec(&analysis).map_err(|e| format!("serialize error: {e}"))
}

fn analyze_response(
    bytes: &[u8],
    replay_id: &str,
    opts: &AnalyzeOpts,
    state: &AppState,
    account: &AccountId,
    team: Option<&Team>,
) -> Response {
    match analyze_json(bytes, replay_id, opts, state, account, team) {
        Ok(json) => Response::json(json),
        Err(e) => Response::text(400, e),
    }
}

/// `POST /api/jobs`: queue the analysis on a worker thread and answer `202` with the job id.
fn submit_job(
    req: &Request,
    state: &Arc<AppState>,
    account: &AccountId,
    team: Option<Team>,
) -> Response {
    let name = query_param(&req.path, "name").unwrap_or_else(|| "upload".to_string());
    queue_job(
        req.body.clone(),
        name,
        AnalyzeOpts::from_request(req),
        state,
        account.clone(),
        team,
    )
}

fn queue_job(
    bytes: Vec<u8>,
    name: String,
    opts: AnalyzeOpts,
    state: &Arc<AppState>,
    account: AccountId,
    team: Option<Team>,
) -> Response {
    if bytes.is_empty() {
        return Response::text(400, "empty request body — no replay bytes");
    }
    let Some(id) = state.jobs.submit(&account) else {
        return Response::text(
            429,
            "too many analyses in progress — retry when one finishes",
        );
    };
    let (state, job) = (Arc::clone(state), id.clone());
    thread::spawn(move || {
        state.jobs.start(&job);
        let outcome = analyze_json(&bytes, &stem(&name), &opts, &state, &account, team.as_ref());
        state.jobs.finish(&job, outcome);
    });
    let mut resp = json_response(&serde_json::json!({ "job": id, "state": JobState::Queued }));
    resp.status = 202;
    resp
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `POST /api/uploads?size=&sha256=[&name=&rank=&session=&team=]`: a signed URL to `PUT` the
/// replay to, without credentials, valid once for [`TTL_S`] seconds.
fn issue_upload(req: &Request, state: &AppState, account: &AccountId) -> Response {
    let Some(signer) = &state.signer else {
        return Response::text(501, "signed uploads are unavailable (no OS randomness)");
    };
    let size = query_param(&req.path, "size").and_then(|s| s.parse::<usize>().ok());
    let sha = query_param(&req.path, "sha256").map(|s| s.to_ascii_lowercase());
    let (Some(size), Some(sha256)) = (size, sha) else {
        return Response::text(400, "needs ?size=<bytes>&sha256=<hex>");
    };
    if size == 0 {
        return Response::text(400, "size must be at least 1 byte");
    }
    if size > server::MAX_BODY {
        return Response::text(
            413,
            format!("size must be at most {} bytes", server::MAX_BODY),
        );
    }
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Response::text(400, "sha256 must be 64 hex digits");
    }
    let team = match upload_team(req, state, account) {
        Ok(t) => t.map(|t| t.id.to_string()),
        Err(resp) => return resp,
    };
    let opts = AnalyzeOpts::from_request(req);
    let nonce = u64::from_le_bytes(sha256_nonce(&sha256, account.as_str()));
    let upload = Upload {
        account: account.as_str().into(),
        size,
        sha256,
        name: query_param(&req.path, "name").unwrap_or_else(|| "upload".into()),
        rank: opts.rank,
        session: opts.session,
        inline: opts.inline,
        team,
        expires: unix_now() + TTL_S,
        nonce,
    };
    json_response(&serde_json::json!({
        "url": format!("/api/uploads/{}", signer.sign(&upload)),
        "expires_in": TTL_S,
    }))
}

/// A nonce unique per issue: the digest of the declared hash, the account, the time and a counter.
fn sha256_nonce(sha: &str, account: &str) -> [u8; 8] {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let d = rleval_app::sha256::sha256(format!("{sha}|{account}|{nanos}|{n}").as_bytes());
    [d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
}

/// `PUT /api/uploads/<token>`: verify the bytes against the signed declaration, then queue the job.
fn redeem_upload(req: &Request, state: &Arc<AppState>, token: &str) -> Response {
    let Some(signer) = &state.signer else {
        return Response::text(501, "signed uploads are unavailable (no OS randomness)");
    };
    let up = match signer.redeem(token, &req.body, unix_now()) {
        Ok(up) => up,
        Err(e) => {
            use rleval_app::uploads::Refusal::*;
            let status = match e {
                Invalid => 403,
                Expired => 410,
                Used => 409,
                SizeMismatch => 400,
                HashMismatch => 422,
            };
            return Response::text(status, e.to_string());
        }
    };
    let Ok(account) = AccountId::new(&up.account) else {
        return Response::text(403, "invalid upload URL");
    };
    let team = up
        .team
        .as_deref()
        .and_then(|id| state.teams.get(id))
        .filter(|t| t.member(&account).is_some())
        .cloned();
    let opts = AnalyzeOpts {
        rank: up.rank,
        inline: up.inline,
        session: up.session,
    };
    queue_job(req.body.clone(), up.name, opts, state, account, team)
}

/// `GET /api/jobs/<id>/events`: Server-Sent Events, one `state` event per change until the job ends.
fn job_events(state: &Arc<AppState>, account: &AccountId, id: &str) -> Response {
    let Some(first) = state.jobs.status(account, id) else {
        return Response::text(404, "no such job");
    };
    let (state, account, id) = (Arc::clone(state), account.clone(), id.to_string());
    Response::events(Box::new(move |w| {
        let mut current = first;
        loop {
            let data = serde_json::to_string(&current).map_err(std::io::Error::other)?;
            write!(w, "event: state\ndata: {data}\n\n")?;
            w.flush()?;
            if current.state.finished() {
                return Ok(());
            }
            match state.jobs.wait(
                &account,
                &id,
                current.state,
                Duration::from_secs(EVENT_KEEPALIVE_S),
            ) {
                Some(next) if next.state != current.state => current = next,
                Some(_) => {
                    write!(w, ": keep-alive\n\n")?;
                    w.flush()?;
                }
                None => return Ok(()),
            }
        }
    }))
}

/// Idle time between SSE keep-alive comments (s).
const EVENT_KEEPALIVE_S: u64 = 15;

/// `GET /api/jobs/<id>[/result]`. `?since=<state>&wait=<secs>` long-polls until the job leaves that state.
fn job_response(req: &Request, state: &AppState, account: &AccountId, rest: &str) -> Response {
    let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if tail == "result" {
        return match (
            state.jobs.result(account, id),
            state.jobs.status(account, id),
        ) {
            (Some(json), _) => Response::json(json.as_slice().to_vec()),
            (None, Some(s)) if s.state == JobState::Failed => {
                Response::text(422, s.error.unwrap_or_default())
            }
            (None, Some(_)) => Response::text(409, "the analysis is not finished"),
            (None, None) => Response::text(404, "no such job"),
        };
    }
    let wait = query_param(&req.path, "wait")
        .and_then(|w| w.parse::<u64>().ok())
        .unwrap_or(0)
        .min(MAX_WAIT_S);
    let since = query_param(&req.path, "since").and_then(|s| JobState::parse(&s));
    let status = match since {
        Some(since) => state
            .jobs
            .wait(account, id, since, Duration::from_secs(wait)),
        None => state.jobs.status(account, id),
    };
    status.map_or_else(|| Response::text(404, "no such job"), |s| json_response(&s))
}

/// Longest a status request may wait for a change (s).
const MAX_WAIT_S: u64 = 25;

/// Save the analysis to the account's history and, when uploading for a team,
/// to that team's shared pool. A storage failure must not lose the user's
/// analysis, so it is reported on stderr and the response still succeeds.
fn persist(
    state: &AppState,
    account: &AccountId,
    team: Option<&Team>,
    bytes: &[u8],
    session: Option<&str>,
    analysis: &pipeline::Analysis,
) {
    let Some(store) = &state.store else { return };
    let saved_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let record = SessionRecord::from_analysis(session_key(bytes), saved_at, analysis)
        .in_session(session.unwrap_or_default());
    save_logged(
        store.as_ref(),
        account,
        &record,
        &format!("{account}'s history"),
    );
    if let (Some(team), Some(pool)) = (team, &state.team_store) {
        save_logged(
            pool.as_ref(),
            team.id.namespace(),
            &record,
            &format!("team {}", team.id),
        );
    }
}

fn save_logged(store: &dyn SessionStore, ns: &AccountId, record: &SessionRecord, whose: &str) {
    match store.save(ns, record) {
        Ok(SaveOutcome::Saved) => eprintln!("history: saved {} to {whose}", record.label),
        Ok(SaveOutcome::AlreadyStored) => {}
        Err(e) => eprintln!("warning: could not save {} to {whose}: {e}", record.label),
    }
}

fn samples_response(replays: &Path) -> Response {
    let mut names: Vec<String> = std::fs::read_dir(replays)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|s| s.to_str()) == Some("replay"))
                .then(|| p.file_name()?.to_str().map(String::from))
                .flatten()
        })
        .collect();
    names.sort();
    let body = serde_json::json!({ "samples": names });
    Response::json(serde_json::to_vec(&body).unwrap_or_default())
}

// ---- analyze (one-shot, no server) ----

fn analyze_oneshot(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let mut replay: Option<String> = None;
    let mut out: Option<String> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(it.next().ok_or("--out needs a path")?),
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other}\n{USAGE}").into())
            }
            other if replay.is_none() => replay = Some(other.to_string()),
            other => return Err(format!("unexpected arg {other}").into()),
        }
    }
    let replay = replay.ok_or(USAGE)?;
    let bytes = std::fs::read(&replay)?;
    let id = stem(&replay);
    // Use the default corpus norms if present, so the static bundle also carries
    // the rank-relative layer; absent ⇒ purely absolute.
    let norms = pipeline::load_rank_norms(Path::new("assets/corpus"));
    let xg = pipeline::load_xg(Path::new("assets/corpus"));
    let analysis = pipeline::analyze(&bytes, &id, norms.as_ref(), None, &xg)?;

    let out = out.unwrap_or_else(|| format!("{id}.html"));
    std::fs::write(&out, bundle_html(&analysis))?;
    eprintln!(
        "analyzed {id}: {} players, {} skills, {} value rows -> {out}",
        analysis.scores.len(),
        analysis.skill_profiles.len(),
        analysis.impact.players.len(),
    );
    Ok(())
}

/// A static, self-contained HTML bundle (no server): the index shell with the
/// analysis JSON inlined so it renders immediately when opened in a browser.
fn bundle_html(analysis: &pipeline::Analysis) -> String {
    let json = serde_json::to_string(analysis).unwrap_or_else(|_| "null".into());
    // Splice the data in just before the closing script: define a global the UI
    // picks up, and skip the network fetch.
    let inject = format!(
        "<script>window.__RLEVAL_DATA__ = {json};</script>\n",
        json = json.replace("</script>", "<\\/script>"),
    );
    let mut doc = ui::INDEX_HTML.to_string();
    // Drop the bundled data right before </body>, plus a tiny bootstrap.
    let bootstrap = "<script>if(window.__RLEVAL_DATA__){DATA=window.__RLEVAL_DATA__;render();\
        document.getElementById('loader').style.display='none';}</script>";
    doc = doc.replace("</body>", &format!("{inject}{bootstrap}</body>"));
    doc
}

// ---- small helpers ----

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string()
}

/// Pull a query parameter value out of a raw request path.
fn query_param(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

/// Minimal percent-decoding (enough for filenames and query values).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_strips_dir_and_extension() {
        assert_eq!(stem("assets/replays/42f2.replay"), "42f2");
        assert_eq!(stem("upload"), "upload");
    }

    #[test]
    fn query_param_extracts_and_decodes() {
        assert_eq!(
            query_param("/api/analyze?name=my%20game.replay", "name").as_deref(),
            Some("my game.replay")
        );
        assert_eq!(query_param("/api/analyze", "name"), None);
        assert_eq!(query_param("/x?a=1&b=2", "b").as_deref(), Some("2"));
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("plain"), "plain");
    }
}
