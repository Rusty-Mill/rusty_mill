//! Settings: the admin operations behind the UI's settings view (`POST /api/settings_*`).
//!
//! Agents (credentials), repositories and projects, roles and the GitHub sync live in files the
//! server was pointed at (`--settings-dir`, or one flag per file). A change is validated, written
//! atomically (secrets owner-only), and only then applied to the running board, so the UI and the
//! files never disagree and every file stays human-readable. Each operation needs the `Admin`
//! capability. They are not MCP tools: a model should not mint credentials. The one exception is
//! `agent_enrol` (and `agent_enrol_revoke`), over `/api` only: it needs `Enrol`, mints only within
//! the caller's own role and projects, and only for itself (`lead.label`, at most 8).
//!
//! Apps signed in through OAuth are listed and revoked here too; their grants live in OAuth's own
//! state file. Nothing here ever returns a token except the one it just made, once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use rusty_oauth_server::Provider;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::agents::Grant;
use crate::api::{CallError, Identity, Shared, Who, acting};
use crate::board::{ProjectsSink, Roles};
use crate::domain::{Actor, BoardError, Capability, MoveRules, ProjectStatus};
use crate::github::GithubConfig;
use crate::identities::{Credentials, read_private};
use crate::projects::{Projects, Statuses};
use crate::syncer::{MIN_SECS, Syncer};
use crate::workflow::Workflow;

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
    /// Whether card movement rules are enforced.
    pub rules: Option<PathBuf>,
    /// The columns and the moves between them.
    pub workflow: Option<PathBuf>,
    /// Each project's status (active unless listed).
    pub project_status: Option<PathBuf>,
}

impl Files {
    /// The settings files in one directory.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            identities: Some(dir.join("identities.json")),
            projects: Some(dir.join("projects.json")),
            roles: Some(dir.join("roles.json")),
            github: Some(dir.join("github.json")),
            rules: Some(dir.join("rules.json")),
            workflow: Some(dir.join("workflow.json")),
            project_status: Some(dir.join("project-status.json")),
        }
    }
}

/// How the server was started, shown read-only in the settings view.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ServerInfo {
    /// Where the cards are kept: a rusty_tick URL, or `None` for memory.
    pub tick: Option<String>,
    /// The public origin OAuth sign-in is served under, when on.
    pub issuer: Option<String>,
    /// Accepted `Host` names.
    pub hosts: Vec<String>,
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

/// The rules file: `{"movement": "enforced" | "open"}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesFile {
    /// Whether card moves follow the transition table.
    pub movement: MoveRules,
}

/// The movement rules from `path`, or the default ([`MoveRules::Enforced`]) if there is no such
/// file yet.
///
/// # Errors
/// If it exists but cannot be read or parsed.
pub fn load_rules(path: &Path) -> Result<MoveRules, String> {
    if !path.exists() {
        return Ok(MoveRules::default());
    }
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: RulesFile = serde_json::from_str(&json).map_err(|e| format!("rules: {e}"))?;
    Ok(file.movement)
}

/// The project statuses from `path`, or every project active if there is no such file yet.
///
/// # Errors
/// If it exists but cannot be read or parsed.
pub fn load_statuses(path: &Path) -> Result<Statuses, String> {
    if !path.exists() {
        return Ok(Statuses::default());
    }
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Statuses::parse(&json)
}

/// The workflow from `path`, or the default (the original seven columns) if there is no such
/// file yet.
///
/// # Errors
/// If it exists but cannot be read, parsed or validated.
pub fn load_workflow(path: &Path) -> Result<Workflow, String> {
    if !path.exists() {
        return Ok(Workflow::default());
    }
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&json).map_err(|e| format!("workflow: {e}"))
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

/// `agent_enrol`: `agent_role` and `agent_projects`, because `role` already names the caller's.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enrol {
    label: String,
    agent_role: String,
    agent_projects: Option<Vec<String>>,
    // The caller's own `actor` and `role` (see `Who`), read by `acting`; refused if they differ.
    #[serde(rename = "actor")]
    _actor: Option<String>,
    #[serde(rename = "role")]
    _role: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolLabel {
    label: String,
    #[serde(rename = "actor")]
    _actor: Option<String>,
    #[serde(rename = "role")]
    _role: Option<String>,
}

/// A registered agent's own role, projects and nickname; anything left out is cleared (the agent
/// acts as its connector again, with no nickname).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisteredAgentSet {
    id: String,
    agent_role: Option<String>,
    agent_projects: Option<Vec<String>>,
    nickname: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisteredAgentId {
    id: String,
}

#[derive(Deserialize)]
struct AppId {
    client_id: String,
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
struct RoleRename {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct RulesSet {
    movement: MoveRules,
}

#[derive(Deserialize)]
struct ProjectStatusSet {
    project: String,
    status: ProjectStatus,
}

#[derive(Deserialize)]
struct WorkflowSet {
    workflow: Workflow,
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
    /// OAuth's provider, once the web layer turns sign-in on.
    oauth: OnceLock<Arc<Provider>>,
    server: ServerInfo,
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
            oauth: OnceLock::new(),
            server: ServerInfo::default(),
        }
    }

    /// Show how the server was started.
    #[must_use]
    pub fn with_server(mut self, server: ServerInfo) -> Self {
        self.server = server;
        self
    }

    /// List and revoke the apps signed in through `provider`. Only the first one is kept.
    pub fn attach_oauth(&self, provider: Arc<Provider>) {
        let _ = self.oauth.set(provider);
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
        if matches!(name, "agent_enrol" | "agent_enrol_revoke") {
            return Some(self.enrolling(name, identity, args));
        }
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
            "settings_role_rename" => |s, _, a| s.role_rename(&parse(a)?),
            "settings_github_set" => |s, _, a| s.github_set(&parse(a)?),
            "settings_github_clear" => |s, _, _| s.github_clear(),
            "settings_rules_set" => |s, by, a| s.rules_set(by, &parse(a)?),
            "settings_project_status_set" => |s, by, a| s.project_status_set(by, &parse(a)?),
            "settings_workflow_set" => {
                |s, by, a| s.workflow_set(by, parse::<WorkflowSet>(a)?.workflow)
            }
            "settings_registered_agent_set" => |s, _, a| s.registered_agent_set(&parse(a)?),
            "settings_registered_agent_revoke" => |s, _, a| s.registered_agent_revoke(&parse(a)?),
            "settings_app_revoke" => |s, _, a| s.app_revoke(&parse(a)?),
            _ => return None,
        };
        Some(self.admin(identity, args, run))
    }

    /// `agent_enrol` and `agent_enrol_revoke`: not admin settings, but gated by `Enrol` and bound
    /// to the caller's own credential, which becomes the lead of whatever it mints.
    fn enrolling(
        &self,
        name: &str,
        identity: Option<&Identity>,
        args: Value,
    ) -> Result<Value, CallError> {
        let Some(lead) = identity else {
            return Err(refused("this needs an authenticated credential").into());
        };
        let who: Who =
            serde_json::from_value(args.clone()).map_err(|e| CallError::BadArgs(e.to_string()))?;
        let actor = acting(&self.board, identity, &who)?;
        if !actor.caps.contains(&Capability::Enrol) {
            return Err(BoardError::Forbidden(Capability::Enrol).into());
        }
        let _one_at_a_time = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let out = if name == "agent_enrol" {
            self.enrol(&actor, &lead.actor, &parse(args)?)
        } else {
            self.enrol_revoke(&lead.actor, &parse(args)?)
        };
        out.map_err(CallError::Board)
    }

    /// Mint `lead.label` for the caller, no wider than the caller: a role whose capabilities are
    /// a subset of the caller's and never `Enrol` or `Admin`, and projects within the caller's.
    fn enrol(&self, caller: &Actor, lead: &str, a: &Enrol) -> Res {
        let roles = self.board.roles();
        let caps = roles
            .0
            .get(&a.agent_role)
            .ok_or_else(|| refused(format!("there is no role {:?}", a.agent_role)))?;
        if caps.contains(&Capability::Enrol) || caps.contains(&Capability::Admin) {
            return Err(refused("a minted agent cannot hold enrol or admin"));
        }
        if let Some(extra) = caps.iter().find(|c| !caller.caps.contains(c)) {
            return Err(refused(format!(
                "you do not hold {extra:?}, so you cannot mint it"
            )));
        }
        let projects = match (&caller.projects, &a.agent_projects) {
            (Some(_), None) => {
                return Err(refused(
                    "you are limited to some projects: name agent_projects",
                ));
            }
            (Some(own), Some(asked)) => {
                if let Some(p) = asked.iter().find(|p| !own.contains(*p)) {
                    return Err(refused(format!("you do not work in project {p:?}")));
                }
                a.agent_projects.clone()
            }
            (None, _) => a.agent_projects.clone(),
        };
        self.check_grant(&a.agent_role, projects.as_deref())?;
        let token = self.change_creds(|c| {
            c.add_child(lead, &a.label, a.agent_role.clone(), projects.clone())
        })?;
        rusty_log::warn!(lead = %lead, label = %a.label, role = %a.agent_role, projects = ?projects, "agent minted");
        Ok(json!({ "token": token, "agent": format!("{lead}.{}", a.label) }))
    }

    /// Remove minted agents that no longer fit under their lead (after a lead or roles change).
    fn prune_minted(&self) -> Result<Vec<String>, BoardError> {
        let roles = self.board.roles();
        let due = self.creds.as_ref().is_some_and(|c| {
            !c.read()
                .unwrap_or_else(PoisonError::into_inner)
                .exceeding(&roles)
                .is_empty()
        });
        if !due {
            return Ok(Vec::new());
        }
        let gone = self.change_creds(|c| Ok(c.prune_children(&roles)))?;
        rusty_log::warn!(removed = ?gone, "minted agents removed: they exceeded their lead");
        Ok(gone)
    }

    /// Remove one agent the caller minted, and nothing else.
    fn enrol_revoke(&self, lead: &str, a: &EnrolLabel) -> Res {
        let child = format!("{lead}.{}", a.label);
        self.change_creds(|c| match c.parent_of(&child) {
            Some(p) if p == lead => c.remove(&child),
            _ => Err(format!("you did not mint {child:?}")),
        })?;
        rusty_log::warn!(lead = %lead, agent = %child, "minted agent revoked");
        Ok(json!({ "ok": true }))
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
            let creds = c.read().unwrap_or_else(PoisonError::into_inner);
            creds
                .list()
                .map(|i| {
                    let mut agent =
                        json!({ "actor": i.actor, "role": i.role, "projects": i.projects });
                    if let Some(lead) = creds.parent_of(&i.actor) {
                        agent["parent"] = json!(lead);
                    }
                    agent
                })
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
                "rules": self.files.rules.is_some(),
                "workflow": self.files.workflow.is_some(),
                "project_status": self.files.project_status.is_some(),
            },
            "agents": agents,
            "repos": self.board.projects(),
            "roles": self.board.roles(),
            "capabilities": Capability::ALL,
            "capability_about": Capability::ALL.map(|c| (c, c.about())).into_iter().collect::<BTreeMap<_, _>>(),
            "github": github,
            "rules": RulesFile { movement: self.board.move_rules() },
            "workflow": self.board.workflow(),
            "project_status": self.board.statuses(),
            "apps": self.oauth.get().map(|p| p.store().list_clients()),
            "registered_agents": self.board.agent_list(),
            "server": self.server,
            "paths": {
                "agents": self.files.identities,
                "projects": self.files.projects,
                "roles": self.files.roles,
                "github": self.files.github,
                "rules": self.files.rules,
                "workflow": self.files.workflow,
                "project_status": self.files.project_status,
            },
        }))
    }

    /// Give a registered agent its own role and projects (an admin may widen as well as narrow);
    /// with neither, it acts as its connector again.
    fn registered_agent_set(&self, a: &RegisteredAgentSet) -> Res {
        let grant = Grant {
            role: a.agent_role.clone(),
            projects: a.agent_projects.clone(),
        };
        self.board.check_agent_grant(&grant, None)?;
        // The nickname first: a bad one then changes nothing.
        let known = self.board.set_agent_nickname(&a.id, a.nickname.clone())?;
        if known && self.board.set_agent_grant(&a.id, grant)? {
            rusty_log::warn!(agent = %a.id, role = ?a.agent_role, projects = ?a.agent_projects, "registered agent regranted");
            Ok(json!({"id": a.id, "agent_role": a.agent_role, "agent_projects": a.agent_projects}))
        } else {
            Err(refused(format!("no registered agent {:?}", a.id)))
        }
    }

    /// Forget a registered agent: its key stops working at once; it can register again.
    fn registered_agent_revoke(&self, a: &RegisteredAgentId) -> Res {
        if self.board.revoke_agent(&a.id)? {
            Ok(json!({"revoked": a.id}))
        } else {
            Err(refused(format!("no registered agent {:?}", a.id)))
        }
    }

    /// Sign an app out: drop its registration and every token it holds. It can sign in again.
    fn app_revoke(&self, a: &AppId) -> Res {
        let provider = self
            .oauth
            .get()
            .ok_or_else(|| refused("OAuth sign-in is off: start the board with --issuer"))?;
        match provider.store().revoke_client(&a.client_id) {
            Ok(Some(gone)) => Ok(json!(gone)),
            Ok(None) => Err(refused(format!("no app {:?}", a.client_id))),
            Err(e) => Err(refused(format!("not revoked: {e}"))),
        }
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
        self.fits_saved_roles(&next)?;
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
            connector: false,
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
        let pruned = self.prune_minted()?;
        Ok(json!({ "ok": true, "removed": pruned }))
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
        self.write_roles(&a.roles)?;
        let pruned = self.prune_minted()?;
        Ok(json!({ "roles": a.roles, "removed": pruned }))
    }

    /// Validate `roles` against the agents, write them to their file, then apply them.
    fn write_roles(&self, roles: &Roles) -> Result<(), BoardError> {
        let path = self.file(&self.files.roles)?;
        check_roles(roles).map_err(refused)?;
        if let Some(creds) = &self.creds {
            let creds = creds.read().unwrap_or_else(PoisonError::into_inner);
            if let Some(i) = creds.list().find(|i| !roles.0.contains_key(&i.role)) {
                return Err(refused(format!(
                    "agent {:?} has role {:?}, which would no longer exist; change the agent first",
                    i.actor, i.role
                )));
            }
            self.keeps_an_admin(&creds, roles)?;
        }
        self.saved_creds_fit(roles)?;
        let json = serde_json::to_string_pretty(roles).map_err(|e| refused(e.to_string()))?;
        rusty_atomic_file::write(&path, json.as_bytes())
            .map_err(|e| refused(format!("{}: {e}", path.display())))?;
        self.board.set_roles(roles.clone());
        Ok(())
    }

    /// The credentials file as saved must still load with `roles`. A failed save does not
    /// prove the file is unchanged (an atomic write can fail after replacing it), so the saved
    /// file may hold roles the live set does not.
    fn saved_creds_fit(&self, roles: &Roles) -> Result<(), BoardError> {
        let Some(path) = self
            .files
            .identities
            .as_ref()
            .filter(|_| self.creds.is_some())
        else {
            return Ok(());
        };
        if !path.exists() {
            return Ok(());
        }
        let saved = Credentials::load(path, roles, &self.board.projects()).map_err(|e| {
            refused(format!(
                "the saved credentials would not load with these roles ({e}); save any agent on the Agents tab first"
            ))
        })?;
        self.keeps_an_admin(&saved, roles)
    }

    /// `creds` must load with the roles as saved, the other half of [`Self::saved_creds_fit`]:
    /// a failed roles write may still have replaced the file. No roles file: the defaults,
    /// which is what a restart reads.
    fn fits_saved_roles(&self, creds: &Credentials) -> Result<(), BoardError> {
        let saved = match &self.files.roles {
            Some(path) if path.exists() => load_roles(path).map_err(refused)?,
            Some(_) => Roles::default(),
            None => self.board.roles(),
        };
        if let Some(i) = creds.list().find(|i| !saved.0.contains_key(&i.role)) {
            return Err(refused(format!(
                "agent {:?} would have role {:?}, which the saved roles file lacks; save the roles on the Roles tab first",
                i.actor, i.role
            )));
        }
        self.keeps_an_admin(creds, &saved)
    }

    /// Rename a role and move every agent holding it: add the new name beside the old, move
    /// the agents, drop the old name. Each step leaves the files valid whatever a failed write
    /// left behind, so nothing is rolled back: a failure keeps both names live (on disk the
    /// old one may already be gone), and the error says how to finish from there. Every later
    /// change is checked against the saved files as well as the live state, so none can make
    /// them unloadable in the meantime.
    fn role_rename(&self, a: &RoleRename) -> Res {
        let mut roles = self.board.roles();
        if roles.0.contains_key(&a.to) {
            return Err(refused(format!("there is already a role {:?}", a.to)));
        }
        let caps = roles
            .0
            .get(&a.from)
            .cloned()
            .ok_or_else(|| refused(format!("there is no role {:?}", a.from)))?;
        roles.0.insert(a.to.clone(), caps);
        self.write_roles(&roles)?;
        if self.held(&a.from) {
            self.change_creds(|c| {
                c.rename_role(&a.from, &a.to);
                Ok(())
            })
            .map_err(|e| {
                refused(format!(
                    "partly renamed: roles {:?} and {:?} both exist with the same capabilities, but the agents \
                     may not have moved ({e}). To finish, give {:?}'s agents role {:?} on the Agents tab, then remove {:?}",
                    a.from, a.to, a.from, a.to, a.from
                ))
            })?;
        }
        roles.0.remove(&a.from);
        self.write_roles(&roles).map_err(|e| {
            refused(format!(
                "partly renamed: the agents hold {:?}, but dropping {:?} may not have been saved ({e}). \
                 To finish, remove {:?} on the Roles tab",
                a.to, a.from, a.from
            ))
        })?;
        Ok(json!({ "roles": roles }))
    }

    /// Whether some agent holds `role`.
    fn held(&self, role: &str) -> bool {
        self.creds.as_ref().is_some_and(|c| {
            c.read()
                .unwrap_or_else(PoisonError::into_inner)
                .list()
                .any(|i| i.role == role)
        })
    }

    /// Turn the movement rules on or off: written first, then applied, and logged.
    fn rules_set(&self, by: &Actor, a: &RulesSet) -> Res {
        let path = self.file(&self.files.rules)?;
        let file = RulesFile {
            movement: a.movement,
        };
        let json = serde_json::to_string_pretty(&file).map_err(|e| refused(e.to_string()))?;
        rusty_atomic_file::write(&path, json.as_bytes())
            .map_err(|e| refused(format!("{}: {e}", path.display())))?;
        self.board.set_move_rules(a.movement);
        rusty_log::warn!(actor = %by.name, movement = ?a.movement, "card movement rules changed");
        Ok(json!(file))
    }

    /// Set one project's status: written under the board's lock before it applies, and logged.
    fn project_status_set(&self, by: &Actor, a: &ProjectStatusSet) -> Res {
        let path = self.file(&self.files.project_status)?;
        let statuses = self
            .board
            .set_project_status(&a.project, a.status, |next| {
                let json = serde_json::to_string_pretty(next).map_err(|e| e.to_string())?;
                rusty_atomic_file::write(&path, json.as_bytes())
                    .map_err(|e| format!("{}: {e}", path.display()))
            })?;
        rusty_log::warn!(actor = %by.name, project = %a.project, status = a.status.as_str(), "project status changed");
        Ok(json!({ "project_status": statuses }))
    }

    /// Replace the columns and moves. Refused if a removed column still holds cards; the file
    /// is written under the board's lock, before the change applies, and the change is logged.
    fn workflow_set(&self, by: &Actor, next: Workflow) -> Res {
        let path = self.file(&self.files.workflow)?;
        let json = serde_json::to_string_pretty(&next).map_err(|e| refused(e.to_string()))?;
        self.board.set_workflow(next, |_| {
            rusty_atomic_file::write(&path, json.as_bytes())
                .map_err(|e| format!("{}: {e}", path.display()))
        })?;
        let flow = self.board.workflow();
        let columns: Vec<&str> = flow.columns().iter().map(|c| c.id.as_str()).collect();
        rusty_log::warn!(actor = %by.name, columns = ?columns, "board workflow changed");
        Ok(json!({ "workflow": flow }))
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
