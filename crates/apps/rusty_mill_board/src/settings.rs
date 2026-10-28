//! Settings: the admin operations behind the UI's settings view (`POST /api/settings_*`).
//!
//! Agents (credentials), repositories and projects, roles and the GitHub sync live in files the
//! server was pointed at (`--settings-dir`, or one flag per file). A change is validated, written
//! atomically (secrets owner-only), and only then applied to the running board, so the UI and the
//! files never disagree and every file stays human-readable. Each operation needs the `Admin`
//! capability. They are not MCP tools: a model should not mint credentials.
//!
//! Nothing here ever returns a token except the one it just made, once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::api::{CallError, Identity, Shared, Who, acting};
use crate::board::{ProjectsSink, Roles};
use crate::domain::{Actor, BoardError, Capability};
use crate::github::GithubConfig;
use crate::identities::{Credentials, read_private};
use crate::projects::Projects;
use crate::syncer::{MIN_SECS, Syncer};

const DEFAULT_API: &str = "https://api.github.com";
const DEFAULT_SECS: u64 = 60;
const HINT: &str =
    "start the board with --settings-dir DIR (or the matching file flag) to manage this here";

/// Where each kind of setting is kept. `None`: not configured, so read-only.
#[derive(Debug, Clone, Default)]
pub struct Files {
    /// Credentials: a token per agent, with its role and project limit.
    pub identities: Option<PathBuf>,
    /// Repositories and projects.
    pub projects: Option<PathBuf>,
    /// Roles and their capabilities.
    pub roles: Option<PathBuf>,
    /// The GitHub sync.
    pub github: Option<PathBuf>,
}

impl Files {
    /// The four files in one directory.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            identities: Some(dir.join("identities.json")),
            projects: Some(dir.join("projects.json")),
            roles: Some(dir.join("roles.json")),
            github: Some(dir.join("github.json")),
        }
    }
}

/// The GitHub sync as its file holds it. The token is a secret: the file is owner-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubFile {
    /// `owner/name`.
    pub repo: String,
    /// REST base URL.
    #[serde(default = "default_api")]
    pub api: String,
    /// Seconds between runs.
    #[serde(default = "default_secs")]
    pub interval_secs: u64,
    /// The access token.
    pub token: String,
}

fn default_api() -> String {
    DEFAULT_API.to_owned()
}

fn default_secs() -> u64 {
    DEFAULT_SECS
}

impl GithubFile {
    /// The validated client configuration.
    ///
    /// # Errors
    /// If the repo, API base or token is invalid, or the interval is too short.
    pub fn config(&self) -> Result<GithubConfig, String> {
        if self.interval_secs < MIN_SECS {
            return Err(format!("the interval must be at least {MIN_SECS} seconds"));
        }
        GithubConfig::new(&self.api, &self.repo, &self.token)
    }
}

/// Roles from a file.
///
/// # Errors
/// If it cannot be read or parsed, or no role holds `Admin` (nobody could ever change settings).
pub fn load_roles(path: &Path) -> Result<Roles, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let roles: Roles = serde_json::from_str(&json).map_err(|e| format!("roles: {e}"))?;
    check_roles(&roles)?;
    Ok(roles)
}

/// Roles must be plainly named, and some role must hold `Admin`.
/// Persist a project registry to the projects file at `path`, atomically.
pub fn projects_sink(path: PathBuf) -> ProjectsSink {
    Box::new(move |projects: &Projects| {
        let json = projects.file_json()?;
        rusty_atomic_file::write(&path, json.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    })
}

fn check_roles(roles: &Roles) -> Result<(), String> {
    let plain = |n: &str| {
        !n.is_empty()
            && n.len() <= 40
            && n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    };
    if let Some(bad) = roles.0.keys().find(|n| !plain(n)) {
        return Err(format!("role name {bad:?} must be 1-40 of a-z 0-9 _ -"));
    }
    if !roles
        .0
        .values()
        .any(|caps| caps.contains(&Capability::Admin))
    {
        return Err(
            "some role must hold the admin capability, or nobody could change settings".into(),
        );
    }
    Ok(())
}

/// The credentials at `path`. If there is no such file yet, one is created holding a first admin
/// (named `admin`, role `human`), and its token is returned: it is shown once and never again.
///
/// # Errors
/// If the file exists but is group/world-readable or invalid, there is no `human` role holding
/// `Admin` to give the first admin, or the file cannot be created.
pub fn open_credentials(
    path: &Path,
    roles: &Roles,
    projects: &Projects,
) -> Result<(Credentials, Option<String>), String> {
    if path.exists() {
        return Ok((Credentials::load(path, roles, projects)?, None));
    }
    if !roles
        .0
        .get("human")
        .is_some_and(|caps| caps.contains(&Capability::Admin))
    {
        return Err(format!(
            "{} does not exist, and there is no `human` role with the admin capability to give its first agent",
            path.display()
        ));
    }
    let (creds, token) = Credentials::first_admin("human")?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    creds.save(path)?;
    Ok((creds, Some(token)))
}

/// The GitHub file, if there is one.
///
/// # Errors
/// If it exists but is group/world-readable, unreadable or invalid.
pub fn load_github(path: &Path) -> Result<Option<GithubFile>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let file: GithubFile =
        serde_json::from_str(&read_private(path)?).map_err(|e| format!("github: {e}"))?;
    file.config()?;
    Ok(Some(file))
}

/// `Some(None)` when the key is present and null; `None` when absent.
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(d)?))
}

/// `{"agent": {...}}`: the new agent's `role` must not be confused with the caller's own.
#[derive(Deserialize)]
struct WithAgent<A> {
    agent: A,
}

#[derive(Deserialize)]
struct AgentAdd {
    name: String,
    role: String,
    projects: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct AgentUpdate {
    name: String,
    role: Option<String>,
    /// Absent: unchanged. `null`: lift the limit. A list: limit to those projects.
    #[serde(default, deserialize_with = "present")]
    projects: Option<Option<Vec<String>>>,
}

#[derive(Deserialize)]
struct AgentName {
    name: String,
}

#[derive(Deserialize)]
struct ProjectsSet {
    repos: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

#[derive(Deserialize)]
struct RolesSet {
    roles: Roles,
}

#[derive(Deserialize)]
struct GithubSet {
    repo: String,
    api: Option<String>,
    interval_secs: Option<u64>,
    /// Absent or blank: keep the stored token.
    token: Option<String>,
}

type Res = Result<Value, BoardError>;

fn refused(why: impl Into<String>) -> BoardError {
    BoardError::Refused(why.into())
}

/// The settings service.
pub struct Settings {
    board: Arc<Shared>,
    files: Files,
    creds: Option<Arc<RwLock<Credentials>>>,
    syncer: Syncer,
    /// The sync as configured now (file, or the flags it started from), token included.
    github: Mutex<Option<GithubFile>>,
    /// One change at a time.
    changing: Mutex<()>,
}

impl Settings {
    /// `creds` is the set the web layer authenticates against (present when the board runs with
    /// per-model credentials); `github` is the sync's starting configuration, already running.
    pub fn new(
        board: Arc<Shared>,
        files: Files,
        creds: Option<Arc<RwLock<Credentials>>>,
        syncer: Syncer,
        github: Option<GithubFile>,
    ) -> Self {
        Self {
            board,
            files,
            creds,
            syncer,
            github: Mutex::new(github),
            changing: Mutex::new(()),
        }
    }

    /// Settings with nothing configured: every change is refused with the way to enable it.
    pub fn read_only(board: Arc<Shared>) -> Self {
        let syncer = Syncer::new(Arc::clone(&board));
        Self::new(board, Files::default(), None, syncer, None)
    }

    /// Run a settings operation, or `None` if `name` is not one.
    pub fn call(
        &self,
        name: &str,
        identity: Option<&Identity>,
        args: Value,
    ) -> Option<Result<Value, CallError>> {
        let run: fn(&Self, &Actor, Value) -> Res = match name {
            "settings_get" => |s, _, _| s.get(),
            "settings_agent_add" => |s, _, a| s.agent_add(&parse::<WithAgent<AgentAdd>>(a)?.agent),
            "settings_agent_update" => {
                |s, _, a| s.agent_update(&parse::<WithAgent<AgentUpdate>>(a)?.agent)
            }
            "settings_agent_remove" => |s, _, a| s.agent_remove(&parse(a)?),
            "settings_agent_rotate" => |s, _, a| s.agent_rotate(&parse(a)?),
            "settings_projects_set" => |s, _, a| s.projects_set(&parse(a)?),
            "settings_roles_set" => |s, _, a| s.roles_set(&parse(a)?),
            "settings_github_set" => |s, _, a| s.github_set(&parse(a)?),
            "settings_github_clear" => |s, _, _| s.github_clear(),
            _ => return None,
        };
        Some(self.admin(identity, args, run))
    }

    fn admin(
        &self,
        identity: Option<&Identity>,
        args: Value,
        run: fn(&Self, &Actor, Value) -> Res,
    ) -> Result<Value, CallError> {
        let who: Who =
            serde_json::from_value(args.clone()).map_err(|e| CallError::BadArgs(e.to_string()))?;
        let actor = acting(&self.board, identity, &who)?;
        if !actor.caps.contains(&Capability::Admin) {
            return Err(BoardError::Forbidden(Capability::Admin).into());
        }
        let _one_at_a_time = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        run(self, &actor, args).map_err(CallError::Board)
    }

    fn get(&self) -> Res {
        let agents: Vec<Value> = self.creds.as_ref().map_or_else(Vec::new, |c| {
            c.read()
                .unwrap_or_else(PoisonError::into_inner)
                .list()
                .map(|i| json!({ "actor": i.actor, "role": i.role, "projects": i.projects }))
                .collect()
        });
        let github = self.github.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(|g| {
            json!({ "repo": g.repo, "api": g.api, "interval_secs": g.interval_secs, "has_token": !g.token.is_empty(),
                    "running": self.syncer.repo().is_some() })
        });
        Ok(json!({
            "files": {
                "agents": self.creds.is_some() && self.files.identities.is_some(),
                "projects": self.files.projects.is_some(),
                "roles": self.files.roles.is_some(),
                "github": self.files.github.is_some(),
            },
            "agents": agents,
            "repos": self.board.projects(),
            "roles": self.board.roles(),
            "capabilities": Capability::ALL,
            "github": github,
        }))
    }

    /// The role must exist and every named project too.
    fn check_grant(&self, role: &str, projects: Option<&[String]>) -> Result<(), BoardError> {
        if !self.board.roles().0.contains_key(role) {
            return Err(refused(format!("there is no role {role:?}")));
        }
        if let Some(projects) = projects {
            if projects.is_empty() {
                return Err(refused(
                    "a project limit must name at least one project (omit it for all)",
                ));
            }
            let known = self.board.projects();
            if let Some(p) = projects.iter().find(|p| !known.contains(p)) {
                return Err(refused(format!("there is no project {p:?}")));
            }
        }
        Ok(())
    }

    /// Change the credentials: on a copy, which must keep someone able to change settings, is
    /// saved, and only then swapped in.
    fn change_creds<T>(
        &self,
        f: impl FnOnce(&mut Credentials) -> Result<T, String>,
    ) -> Result<T, BoardError> {
        let (Some(creds), Some(path)) = (&self.creds, &self.files.identities) else {
            return Err(refused(format!(
                "agents are kept in a credentials file; {HINT}"
            )));
        };
        let mut live = creds.write().unwrap_or_else(PoisonError::into_inner);
        let mut next = live.clone();
        let out = f(&mut next).map_err(refused)?;
        self.keeps_an_admin(&next, &self.board.roles())?;
        next.save(path).map_err(refused)?;
        *live = next;
        Ok(out)
    }

    fn keeps_an_admin(&self, creds: &Credentials, roles: &Roles) -> Result<(), BoardError> {
        let ok = creds.list().any(|i| {
            roles
                .0
                .get(&i.role)
                .is_some_and(|caps| caps.contains(&Capability::Admin))
        });
        ok.then_some(())
            .ok_or_else(|| refused("that would leave no agent able to change settings"))
    }

    fn agent_add(&self, a: &AgentAdd) -> Res {
        self.check_grant(&a.role, a.projects.as_deref())?;
        let identity = Identity {
            actor: a.name.clone(),
            role: a.role.clone(),
            projects: a.projects.clone(),
        };
        let token = self.change_creds(|c| c.add(identity.clone()))?;
        Ok(json!({ "token": token, "agent": identity }))
    }

    fn agent_update(&self, a: &AgentUpdate) -> Res {
        let current_role = self
            .creds
            .as_ref()
            .and_then(|c| {
                let creds = c.read().unwrap_or_else(PoisonError::into_inner);
                creds
                    .list()
                    .find(|i| i.actor == a.name)
                    .map(|i| i.role.clone())
            })
            .ok_or_else(|| refused(format!("no agent {:?}", a.name)))?;
        let role = a.role.clone().unwrap_or(current_role);
        self.check_grant(&role, a.projects.as_ref().and_then(Option::as_deref))?;
        self.change_creds(|c| c.update(&a.name, Some(role), a.projects.clone()))?;
        Ok(json!({ "ok": true }))
    }

    fn agent_remove(&self, a: &AgentName) -> Res {
        self.change_creds(|c| c.remove(&a.name))?;
        Ok(json!({ "ok": true }))
    }

    fn agent_rotate(&self, a: &AgentName) -> Res {
        let token = self.change_creds(|c| c.rotate(&a.name))?;
        Ok(json!({ "token": token }))
    }

    fn file(&self, path: &Option<PathBuf>) -> Result<PathBuf, BoardError> {
        path.clone()
            .ok_or_else(|| refused(format!("this is not kept in a file; {HINT}")))
    }

    fn projects_set(&self, a: &ProjectsSet) -> Res {
        let path = self.file(&self.files.projects)?;
        let new = Projects::from_listing(a.repos.clone()).map_err(refused)?;
        if let Some(creds) = &self.creds {
            let creds = creds.read().unwrap_or_else(PoisonError::into_inner);
            if let Some((who, p)) = creds.list().find_map(|i| {
                i.projects
                    .iter()
                    .flatten()
                    .find(|p| !new.contains(p))
                    .map(|p| (i.actor.clone(), p.clone()))
            }) {
                return Err(refused(format!(
                    "agent {who:?} is limited to project {p:?}, which would no longer exist; change the agent first"
                )));
            }
        }
        // Persisted inside the board's critical section: live and disk change together or not at all.
        self.board.set_projects(new.clone(), projects_sink(path))?;
        Ok(json!({ "repos": new }))
    }

    fn roles_set(&self, a: &RolesSet) -> Res {
        let path = self.file(&self.files.roles)?;
        check_roles(&a.roles).map_err(refused)?;
        if let Some(creds) = &self.creds {
            let creds = creds.read().unwrap_or_else(PoisonError::into_inner);
            if let Some(i) = creds.list().find(|i| !a.roles.0.contains_key(&i.role)) {
                return Err(refused(format!(
                    "agent {:?} has role {:?}, which would no longer exist; change the agent first",
                    i.actor, i.role
                )));
            }
            self.keeps_an_admin(&creds, &a.roles)?;
        }
        let json = serde_json::to_string_pretty(&a.roles).map_err(|e| refused(e.to_string()))?;
        rusty_atomic_file::write(&path, json.as_bytes())
            .map_err(|e| refused(format!("{}: {e}", path.display())))?;
        self.board.set_roles(a.roles.clone());
        Ok(json!({ "roles": a.roles }))
    }

    fn github_set(&self, a: &GithubSet) -> Res {
        let path = self.file(&self.files.github)?;
        let current = self
            .github
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let token = a
            .token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .or_else(|| current.as_ref().map(|c| c.token.clone()))
            .ok_or_else(|| refused("a GitHub token is required"))?;
        let file = GithubFile {
            repo: a.repo.trim().to_owned(),
            api: a
                .api
                .clone()
                .filter(|u| !u.trim().is_empty())
                .or_else(|| current.as_ref().map(|c| c.api.clone()))
                .unwrap_or_else(default_api),
            interval_secs: a
                .interval_secs
                .or_else(|| current.as_ref().map(|c| c.interval_secs))
                .unwrap_or(DEFAULT_SECS),
            token,
        };
        let config = file.config().map_err(refused)?;
        let json = serde_json::to_string_pretty(&file).map_err(|e| refused(e.to_string()))?;
        rusty_atomic_file::write_private(&path, json.as_bytes())
            .map_err(|e| refused(format!("{}: {e}", path.display())))?;
        self.syncer
            .start(config, &file.repo, Duration::from_secs(file.interval_secs))
            .map_err(|e| refused(format!("saved, but the sync did not start: {e}")))?;
        *self.github.lock().unwrap_or_else(PoisonError::into_inner) = Some(file);
        self.get().map(|v| v["github"].clone())
    }

    fn github_clear(&self) -> Res {
        let path = self.file(&self.files.github)?;
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(refused(format!("{}: {e}", path.display()))),
        }
        self.syncer.stop();
        *self.github.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(json!({ "ok": true }))
    }
}

/// Arguments as the operation's type, or a bad-arguments error.
fn parse<A: DeserializeOwned>(args: Value) -> Result<A, BoardError> {
    serde_json::from_value(args).map_err(|e| BoardError::Refused(format!("invalid arguments: {e}")))
}
