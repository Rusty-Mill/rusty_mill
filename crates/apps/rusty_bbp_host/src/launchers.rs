//! Opinionated provider launchers over the existing per-turn BBP MCP server.
//! Generic moderator launchers remain available for other providers/policies.
//!
//! Unconfined, a launcher reads its turn's [`Facts`] from the task directory
//! and gives the provider a `bbp mcp` bound to it. Confined (the moderator
//! set [`crate::broker::RELAY_ENV`]), the task directory is out of reach: the
//! moderator writes the facts to a file the harness may read but not write
//! ([`FACTS_ENV`]), and the provider's `bbp mcp` is the relay. The checks and
//! the provider's restrictions are the same either way.
use crate::{broker, mcp::Binding, moderator, open_driver};
use rusty_bbp::{
    ArtifactKind, PrincipalId, PrincipalKind, ReadScope, Role, Sha256, State, TaskId, TaskState,
    TurnId,
};
use rusty_serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The two supported launch contracts; names do not determine BBP role ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// Candidate author (also planning and test verdicts), in a private clone.
    Codex,
    /// Independent final reviewer, with no built-in tools or repository clone.
    Claude,
}
impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Default)]
struct Options {
    model: Option<String>,
    binary: Option<PathBuf>,
    mcp: Option<PathBuf>,
}
impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut args = args.into_iter();
        while let Some(key) = args.next() {
            let value = args
                .next()
                .filter(|v| !v.is_empty() && !v.starts_with('-'))
                .ok_or("flags require a nonempty value; provider flags cannot be forwarded")?;
            let duplicate = match key.as_str() {
                "--model" => out.model.replace(value).is_some(),
                "--provider-bin" => out.binary.replace(value.into()).is_some(),
                "--mcp-config" => out.mcp.replace(value.into()).is_some(),
                _ => return Err(format!("unsupported launcher flag {key}")),
            };
            if duplicate {
                return Err(format!("duplicate launcher flag {key}"));
            }
        }
        Ok(out)
    }
}

fn json(value: &impl rusty_serde::Serialize) -> Result<String, String> {
    rusty_serde::json::to_string(value).map_err(|e| e.to_string())
}

/// Names the moderator-written facts file of a confined launch.
pub const FACTS_ENV: &str = "BBP_LAUNCH_FACTS";

/// The role's provider session, from a `"session": true` launcher.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Session {
    /// A UUID, as `--session-id` and `--resume` take it.
    id: String,
    /// An earlier turn of the role started the conversation.
    resume: bool,
}

impl Session {
    /// `BBP_SESSION_ID` and `BBP_SESSION_RESUME` as the moderator sets them:
    /// both or neither, a lowercase UUID and `yes` or `no`.
    fn parse(id: Option<String>, resume: Option<String>) -> Result<Option<Session>, String> {
        let (id, resume) = match (id, resume) {
            (None, None) => return Ok(None),
            (Some(id), Some(resume)) => (id, resume),
            _ => return Err("BBP_SESSION_ID and BBP_SESSION_RESUME come together".into()),
        };
        let uuid = id.len() == 36
            && id.char_indices().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => c == '-',
                _ => matches!(c, '0'..='9' | 'a'..='f'),
            });
        if !uuid {
            return Err("BBP_SESSION_ID is not a session UUID".into());
        }
        let resume = match resume.as_str() {
            "yes" => true,
            "no" => false,
            _ => return Err("BBP_SESSION_RESUME must be yes or no".into()),
        };
        Ok(Some(Session { id, resume }))
    }

    fn from_env() -> Result<Option<Session>, String> {
        let var = |k: &str| std::env::var_os(k).map(|v| v.to_string_lossy().into_owned());
        Session::parse(var("BBP_SESSION_ID"), var("BBP_SESSION_RESUME"))
    }
}

/// What a launcher checks before it starts its provider, taken from the
/// task's state for one principal's turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    /// The task's repository path.
    pub repo: String,
    /// Whether the task is in review.
    pub review: bool,
    /// Whether the turn is a live turn of the principal's role.
    pub live: bool,
    /// The principal's role, when it is an assigned agent.
    pub role: Option<Capabilities>,
    /// The principal's agent vendor.
    pub vendor: Option<String>,
    /// The vendor of every agent whose role writes candidates.
    pub candidate_vendors: Vec<String>,
}

/// The capabilities of a role that the launch policies depend on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub final_decision: bool,
    pub writes_candidate: bool,
    pub writes_nothing: bool,
    pub scoped_messages: bool,
    pub reads_logs: bool,
}

impl Capabilities {
    fn of(role: &Role) -> Capabilities {
        Capabilities {
            final_decision: role.final_decision,
            writes_candidate: role.write.contains(&ArtifactKind::Candidate),
            writes_nothing: role.write.is_empty(),
            scoped_messages: role.read_messages == ReadScope::Scoped,
            reads_logs: role.read_artifacts.contains(&ArtifactKind::Log),
        }
    }
}

impl Facts {
    /// The facts of `principal`'s turn `turn` in `state`.
    pub fn of(state: &TaskState, principal: &PrincipalId, turn: TurnId) -> Facts {
        let role_id = state.role_of(principal);
        let vendor = |kind: &PrincipalKind| match kind {
            PrincipalKind::Agent { vendor, .. } => Some(vendor.clone()),
            _ => None,
        };
        Facts {
            repo: state.repo.clone(),
            review: state.state == State::Review,
            live: role_id
                .as_ref()
                .is_some_and(|r| state.turns.iter().any(|t| t.id == turn && &t.role == r)),
            role: role_id
                .as_ref()
                .and_then(|r| state.roster.get(r))
                .map(Capabilities::of),
            vendor: state
                .principals
                .get(principal)
                .and_then(|p| vendor(&p.kind)),
            candidate_vendors: state
                .principals
                .values()
                .filter_map(|p| match &p.kind {
                    PrincipalKind::Agent { role, vendor }
                        if state
                            .roster
                            .get(role)
                            .is_some_and(|r| r.write.contains(&ArtifactKind::Candidate)) =>
                    {
                        Some(vendor.clone())
                    }
                    _ => None,
                })
                .collect(),
        }
    }

    /// The facts file of a confined launch.
    fn read(path: &Path) -> Result<Facts, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The text of the facts file.
    pub fn to_json(&self) -> Result<String, String> {
        json(self)
    }
}

/// Refuse a launch whose turn, role or vendor does not fit `provider`.
fn check(provider: Provider, facts: &Facts) -> Result<(), String> {
    let role = facts
        .role
        .as_ref()
        .ok_or("principal is not an assigned agent")?;
    if !facts.live {
        return Err("binding is not the current live turn".into());
    }
    validate_role(provider, role)?;
    if facts.vendor.as_deref() != Some(provider.name()) {
        return Err("principal vendor differs from launcher".into());
    }
    if provider == Provider::Claude {
        if !facts.review {
            return Err("reviewer requires a Review turn".into());
        }
        if facts.candidate_vendors.iter().any(|v| v == "claude") {
            return Err("reviewer vendor also authors candidates".into());
        }
    }
    Ok(())
}

/// How the provider's `bbp mcp` reaches its turn.
#[derive(Clone, Debug)]
enum Channel {
    /// Through the task directory, as the binding, for its one turn.
    Direct(Binding, TurnId),
    /// Confined: through the moderator's relay pipes in `pipes`.
    Relay {
        task: TaskId,
        turn: TurnId,
        pipes: PathBuf,
    },
}

impl Channel {
    /// The moderator's per-turn environment: relayed when it names pipes.
    fn from_env() -> Result<Channel, String> {
        let Some(pipes) = std::env::var_os(broker::RELAY_ENV).map(PathBuf::from) else {
            let binding = Binding::from_env()?;
            let turn = binding
                .turn
                .ok_or("a provider launcher serves one turn: set $BBP_TURN")?;
            return Ok(Channel::Direct(binding, turn));
        };
        let var = |k: &str| std::env::var(k).map_err(|_| format!("missing ${k}"));
        Ok(Channel::Relay {
            task: TaskId(var("BBP_TASK")?),
            turn: TurnId(
                var("BBP_TURN")?
                    .parse()
                    .map_err(|_| "BBP_TURN must be a number".to_owned())?,
            ),
            pipes,
        })
    }

    fn task(&self) -> &TaskId {
        match self {
            Self::Direct(b, _) => &b.task,
            Self::Relay { task, .. } => task,
        }
    }

    fn turn(&self) -> TurnId {
        match self {
            Self::Direct(_, turn) => *turn,
            Self::Relay { turn, .. } => *turn,
        }
    }

    /// The environment of the provider's `bbp mcp`.
    fn env(&self) -> BTreeMap<&'static str, String> {
        match self {
            Self::Direct(b, turn) => BTreeMap::from([
                ("BBP_DIR", b.dir.to_string_lossy().into_owned()),
                ("BBP_TASK", b.task.0.clone()),
                ("BBP_PRINCIPAL", b.principal.0.clone()),
                ("BBP_TURN", turn.0.to_string()),
            ]),
            Self::Relay { pipes, .. } => {
                BTreeMap::from([(broker::RELAY_ENV, pipes.to_string_lossy().into_owned())])
            }
        }
    }
}

fn mcp_json(channel: &Channel, bbp: &Path) -> Result<String, String> {
    Ok(format!(
        "{{\"mcpServers\":{{\"bbp\":{{\"command\":{},\"args\":[\"mcp\"],\"env\":{}}}}}}}",
        json(&bbp.to_string_lossy())?,
        json(&channel.env())?
    ))
}

#[derive(Debug, PartialEq, Eq, rusty_serde::Deserialize)]
#[rusty_serde(deny_unknown_fields)]
struct McpConfig {
    #[rusty_serde(rename = "mcpServers")]
    servers: BTreeMap<String, McpServer>,
}

#[derive(Debug, PartialEq, Eq, rusty_serde::Deserialize)]
#[rusty_serde(deny_unknown_fields)]
struct McpServer {
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

fn validate_config(path: &Path, channel: &Channel, bbp: &Path) -> Result<(), String> {
    // A relayed configuration is the moderator's, beside the workspace; only
    // its content can be checked.
    if let Channel::Direct(b, turn) = channel {
        let expected_path = moderator::mcp_config_path(&b.dir, &b.task, *turn);
        if path.canonicalize().map_err(|e| e.to_string())?
            != expected_path.canonicalize().map_err(|e| e.to_string())?
        {
            return Err("MCP configuration belongs to another task/turn".into());
        }
    }
    let actual: McpConfig =
        rusty_serde::json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let expected: McpConfig =
        rusty_serde::json::from_str(&mcp_json(channel, bbp)?).map_err(|e| e.to_string())?;
    if actual != expected {
        return Err("MCP configuration does not match the current binding".into());
    }
    Ok(())
}

fn validate_role(provider: Provider, role: &Capabilities) -> Result<(), String> {
    let valid = match provider {
        Provider::Codex => !role.final_decision && role.writes_candidate,
        Provider::Claude => {
            role.final_decision && role.scoped_messages && role.writes_nothing && !role.reads_logs
        }
    };
    if !valid {
        return Err("role capabilities do not match the provider launcher policy".into());
    }
    Ok(())
}

fn git(args: &[&str]) -> Result<(), String> {
    let mut command = Command::new("git");
    // Source checkout hooks/global filters must not execute during preparation.
    for (name, _) in std::env::vars_os().filter(|(k, _)| k.to_string_lossy().starts_with("GIT_")) {
        command.env_remove(name);
    }
    let out = command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("preparing author checkout: {}", out.status));
    }
    Ok(())
}

/// The provider's working directory in `root`: empty for the reviewer, a
/// private clone of `repo` for the author. A confined author's clone shares
/// the repository's object store, which its sandbox lets it read but not
/// write; an unconfined one copies it.
fn prepare_cwd(
    root: &Path,
    repo: &str,
    provider: Provider,
    confined: bool,
) -> Result<PathBuf, String> {
    let cwd = root.join("workspace");
    if provider == Provider::Claude {
        std::fs::create_dir(&cwd).map_err(|e| e.to_string())?;
        return Ok(cwd);
    }
    if !Path::new(repo).is_absolute() || !Path::new(repo).is_dir() {
        return Err("author requires a local absolute repository path".into());
    }
    let target = cwd.to_str().ok_or("non-UTF8 author checkout")?;
    let objects = if confined {
        "--shared"
    } else {
        "--no-hardlinks"
    };
    git(&[
        "clone",
        "--quiet",
        objects,
        "--no-checkout",
        "--",
        repo,
        target,
    ])?;
    git(&["-C", target, "checkout", "--quiet", "--detach", "HEAD"])?;
    // Do not provide a push destination to the author.
    git(&["-C", target, "remote", "remove", "origin"])?;
    Ok(cwd)
}

const AUTHOR: &str = include_str!("launchers/author.txt");
const REVIEWER: &str = include_str!("launchers/reviewer.txt");
const REVIEW_TOOLS: &str =
    "mcp__bbp__task_card,mcp__bbp__read,mcp__bbp__get_artifact,mcp__bbp__post";

fn arguments(
    provider: Provider,
    channel: &Channel,
    bbp: &Path,
    config: &Path,
    cwd: &Path,
    model: Option<&str>,
    session: Option<&Session>,
) -> Result<Vec<String>, String> {
    let prompt = format!(
        "BBP task {}, turn {}. Prefix every operation id with t{}-.\n{}",
        channel.task().0,
        channel.turn().0,
        channel.turn().0,
        if provider == Provider::Codex {
            AUTHOR
        } else {
            REVIEWER
        }
    );
    let mut args: Vec<String> = match provider {
        Provider::Codex => [
            "exec",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--strict-config",
            "--sandbox",
            "workspace-write",
            "--json",
            "--color",
            "never",
        ]
        .map(str::to_owned)
        .to_vec(),
        Provider::Claude => [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--tools",
            "",
            "--allowedTools",
            REVIEW_TOOLS,
            "--restricted",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--max-turns",
            "20",
            "--settings",
            "{\"disableAllHooks\":true}",
            "--mcp-config",
        ]
        .map(str::to_owned)
        .to_vec(),
    };
    // A session persists its conversation, under the provider's id for it.
    // Before `--mcp-config`, which ends the fixed flags.
    if provider == Provider::Claude {
        let flags: Vec<String> = match session {
            None => vec!["--no-session-persistence".into()],
            Some(s) if s.resume => vec!["--resume".into(), s.id.clone()],
            Some(s) => vec!["--session-id".into(), s.id.clone()],
        };
        let at = args.len() - 1;
        args.splice(at..at, flags);
    }
    if provider == Provider::Codex {
        let env = channel
            .env()
            .iter()
            .map(|(k, v)| Ok(format!("{}={}", json(k)?, json(v)?)))
            .collect::<Result<Vec<_>, String>>()?
            .join(",");
        let tools = ["task_card", "read", "get_artifact", "put_artifact", "post"]
            .map(|name| format!("{name}={{approval_mode=\"approve\"}}"))
            .join(",");
        for option in [
            "approval_policy=\"never\"".to_owned(),
            "features.hooks=false".into(), "features.plugins=false".into(), "features.multi_agent=false".into(),
            format!("projects.{}.trust_level=\"untrusted\"", json(&cwd.to_string_lossy())?),
            "shell_environment_policy.exclude=[\"BBP_*\",\"GIT_*\",\"GH_TOKEN\",\"GITHUB_TOKEN\",\"OPENAI_API_KEY\",\"ANTHROPIC_API_KEY\"]".into(),
            "shell_environment_policy.set={GIT_CONFIG_NOSYSTEM=\"1\",GIT_CONFIG_GLOBAL=\"/dev/null\"}".into(),
            format!("mcp_servers={{bbp={{command={},args=[\"mcp\"],env={{{env}}},required=true,enabled_tools=[\"task_card\",\"read\",\"get_artifact\",\"put_artifact\",\"post\"],tools={{{tools}}}}}}}", json(&bbp.to_string_lossy())?),
        ] { args.extend(["-c".into(), option]); }
    } else {
        args.push(config.to_string_lossy().into_owned());
    }
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    // Claude's --mcp-config is variadic: the prompt must not become a second
    // config source when no --model follows it.
    args.push("--".into());
    args.push(prompt);
    Ok(args)
}

/// Validate one live turn, prepare private configuration, and exec the provider.
/// Exec preserves the moderator's process-group ownership and provider exit code.
/// Unknown/provider flag forwarding is refused so reviewer policy cannot be undone.
pub fn run(provider: Provider) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("BBP provider launchers require Linux".into());
    }
    let options = Options::parse(std::env::args().skip(1))?;
    let channel = Channel::from_env()?;
    let session = Session::from_env()?;
    // Codex names its sessions itself; resuming one would mean trusting an id
    // read back from the provider's own store.
    if provider == Provider::Codex && session.is_some() {
        return Err(
            "the codex launcher cannot keep a session: codex chooses its own session ids".into(),
        );
    }
    let bbp = PathBuf::from(std::env::var_os("BBP_BIN").ok_or("missing BBP_BIN")?);
    if !bbp.is_absolute() {
        return Err("BBP_BIN must be absolute".into());
    }
    let mcp = options
        .mcp
        .or_else(|| std::env::var_os("BBP_MCP_CONFIG").map(PathBuf::from))
        .ok_or("missing per-turn MCP configuration")?;
    // Keep the moderator's spelling in MCP env (including symlink aliases),
    // while validate_config compares file identity through canonical paths.
    if matches!(&channel, Channel::Direct(b, _) if !b.dir.is_absolute()) {
        return Err("BBP_DIR must be absolute".into());
    }
    validate_config(&mcp, &channel, &bbp)?;
    let (facts, parent) = match &channel {
        Channel::Direct(binding, turn) => {
            let driver = open_driver(&binding.dir, &binding.task)?;
            let facts = Facts::of(&driver.state, &binding.principal, *turn);
            let parent = binding
                .dir
                .join("harness")
                .join(Sha256::of(binding.task.0.as_bytes()).hex());
            (facts, parent)
        }
        Channel::Relay { .. } => {
            let path = std::env::var_os(FACTS_ENV).ok_or(format!("missing ${FACTS_ENV}"))?;
            // The sandbox's own scratch directory, private to this harness.
            (
                Facts::read(Path::new(&path))?,
                std::env::temp_dir().join("bbp-launch"),
            )
        }
    };
    check(provider, &facts)?;
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    // A session's turns share one working directory, which Claude Code keys
    // its conversations by; each turn still starts it empty.
    let root = match &session {
        Some(s) => {
            let root = parent.join(format!("session-{}", s.id));
            if root.exists() {
                std::fs::remove_dir_all(&root)
                    .map_err(|e| format!("clearing {}: {e}", root.display()))?;
            }
            root
        }
        None => parent.join(format!("turn-{}", channel.turn().0)),
    };
    std::fs::create_dir(&root)
        .map_err(|e| format!("turn workspace already exists or cannot be created: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let confined = matches!(channel, Channel::Relay { .. });
    let cwd = prepare_cwd(&root, &facts.repo, provider, confined)?;
    let config = root.join("mcp.json");
    rusty_atomic_file::write_private(&config, mcp_json(&channel, &bbp)?.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(options.binary.unwrap_or_else(|| provider.name().into()));
    command
        .args(arguments(
            provider,
            &channel,
            &bbp,
            &config,
            &cwd,
            options.model.as_deref(),
            session.as_ref(),
        )?)
        .current_dir(cwd)
        .stdin(Stdio::null());
    for (name, _) in std::env::vars_os().filter(|(k, _)| {
        k.to_string_lossy().starts_with("BBP_") || k.to_string_lossy().starts_with("GIT_")
    }) {
        command.env_remove(name);
    }
    for name in [
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "CODEX_API_KEY",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "GH_TOKEN",
        "GITHUB_TOKEN",
    ] {
        command.env_remove(name);
    }
    command
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(format!("executing provider: {}", command.exec()))
    }
    #[cfg(not(unix))]
    Err("provider exec requires Unix".into())
}

#[cfg(test)]
mod tests;
