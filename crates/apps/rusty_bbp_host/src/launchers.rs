//! Opinionated provider launchers over the existing per-turn BBP MCP server.
//! Generic moderator launchers remain available for other providers/policies.
use crate::{mcp::Binding, moderator, open_driver};
use rusty_bbp::{ArtifactKind, PrincipalKind, ReadScope, Role, Sha256, State};
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

fn binding_env(b: &Binding) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("BBP_DIR", b.dir.to_string_lossy().into_owned()),
        ("BBP_TASK", b.task.0.clone()),
        ("BBP_PRINCIPAL", b.principal.0.clone()),
        ("BBP_TURN", b.turn.0.to_string()),
    ])
}

fn mcp_json(b: &Binding, bbp: &Path) -> Result<String, String> {
    Ok(format!(
        "{{\"mcpServers\":{{\"bbp\":{{\"command\":{},\"args\":[\"mcp\"],\"env\":{}}}}}}}",
        json(&bbp.to_string_lossy())?,
        json(&binding_env(b))?
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

fn validate_config(path: &Path, b: &Binding, bbp: &Path) -> Result<(), String> {
    let expected_path = moderator::mcp_config_path(&b.dir, &b.task, b.turn);
    if path.canonicalize().map_err(|e| e.to_string())?
        != expected_path.canonicalize().map_err(|e| e.to_string())?
    {
        return Err("MCP configuration belongs to another task/turn".into());
    }
    let actual: McpConfig =
        rusty_serde::json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let expected: McpConfig =
        rusty_serde::json::from_str(&mcp_json(b, bbp)?).map_err(|e| e.to_string())?;
    if actual != expected {
        return Err("MCP configuration does not match the current binding".into());
    }
    Ok(())
}

fn validate_role(provider: Provider, role: &Role) -> Result<(), String> {
    let valid = match provider {
        Provider::Codex => !role.final_decision && role.write.contains(&ArtifactKind::Candidate),
        Provider::Claude => {
            role.final_decision
                && role.read_messages == ReadScope::Scoped
                && role.write.is_empty()
                && !role.read_artifacts.contains(&ArtifactKind::Log)
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

fn prepare_cwd(root: &Path, repo: &str, provider: Provider) -> Result<PathBuf, String> {
    let cwd = root.join("workspace");
    if provider == Provider::Claude {
        std::fs::create_dir(&cwd).map_err(|e| e.to_string())?;
        return Ok(cwd);
    }
    if !Path::new(repo).is_absolute() || !Path::new(repo).is_dir() {
        return Err("author requires a local absolute repository path".into());
    }
    let target = cwd.to_str().ok_or("non-UTF8 author checkout")?;
    git(&[
        "clone",
        "--quiet",
        "--no-hardlinks",
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
    b: &Binding,
    bbp: &Path,
    config: &Path,
    cwd: &Path,
    model: Option<&str>,
) -> Result<Vec<String>, String> {
    let prompt = format!(
        "BBP task {}, turn {}. Prefix every operation id with t{}-.\n{}",
        b.task.0,
        b.turn.0,
        b.turn.0,
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
            "--no-session-persistence",
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
    if provider == Provider::Codex {
        let env = binding_env(b)
            .iter()
            .map(|(k, v)| Ok(format!("{}={}", json(k)?, json(v)?)))
            .collect::<Result<Vec<_>, String>>()?
            .join(",");
        for option in [
            "approval_policy=\"never\"".to_owned(),
            "features.hooks=false".into(), "features.plugins=false".into(), "features.multi_agent=false".into(),
            format!("projects.{}.trust_level=\"untrusted\"", json(&cwd.to_string_lossy())?),
            "shell_environment_policy.exclude=[\"BBP_*\",\"GH_TOKEN\",\"GITHUB_TOKEN\",\"OPENAI_API_KEY\",\"ANTHROPIC_API_KEY\"]".into(),
            format!("mcp_servers={{bbp={{command={},args=[\"mcp\"],env={{{env}}},required=true,enabled_tools=[\"task_card\",\"read\",\"get_artifact\",\"put_artifact\",\"post\"],default_tools_approval_mode=\"approve\"}}}}", json(&bbp.to_string_lossy())?),
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
    let binding = Binding::from_env()?;
    // Keep the moderator's spelling in MCP env (including symlink aliases),
    // while validate_config compares file identity through canonical paths.
    if !binding.dir.is_absolute() {
        return Err("BBP_DIR must be absolute".into());
    }
    let bbp = PathBuf::from(std::env::var_os("BBP_BIN").ok_or("missing BBP_BIN")?);
    if !bbp.is_absolute() {
        return Err("BBP_BIN must be absolute".into());
    }
    let mcp = options
        .mcp
        .or_else(|| std::env::var_os("BBP_MCP_CONFIG").map(PathBuf::from))
        .ok_or("missing per-turn MCP configuration")?;
    validate_config(&mcp, &binding, &bbp)?;
    let driver = open_driver(&binding.dir, &binding.task)?;
    let role_id = driver
        .state
        .role_of(&binding.principal)
        .ok_or("principal is not an assigned agent")?;
    if !driver
        .state
        .turns
        .iter()
        .any(|t| t.id == binding.turn && t.role == role_id)
    {
        return Err("binding is not the current live turn".into());
    }
    let role = driver.state.roster.get(&role_id).ok_or("missing role")?;
    validate_role(provider, role)?;
    let assigned = driver
        .state
        .principals
        .get(&binding.principal)
        .ok_or("missing principal")?;
    if !matches!(&assigned.kind, PrincipalKind::Agent { vendor, .. } if vendor == provider.name()) {
        return Err("principal vendor differs from launcher".into());
    }
    if provider == Provider::Claude {
        if driver.state.state != State::Review {
            return Err("reviewer requires a Review turn".into());
        }
        for principal in driver.state.principals.values() {
            if let PrincipalKind::Agent { role, vendor } = &principal.kind {
                if vendor == "claude"
                    && driver
                        .state
                        .roster
                        .get(role)
                        .is_some_and(|r| r.write.contains(&ArtifactKind::Candidate))
                {
                    return Err("reviewer vendor also authors candidates".into());
                }
            }
        }
    }
    let parent = binding
        .dir
        .join("harness")
        .join(Sha256::of(binding.task.0.as_bytes()).hex());
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let root = parent.join(format!("turn-{}", binding.turn.0));
    std::fs::create_dir(&root)
        .map_err(|e| format!("turn workspace already exists or cannot be created: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let cwd = prepare_cwd(&root, &driver.state.repo, provider)?;
    let config = root.join("mcp.json");
    rusty_atomic_file::write_private(&config, mcp_json(&binding, &bbp)?.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(options.binary.unwrap_or_else(|| provider.name().into()));
    command
        .args(arguments(
            provider,
            &binding,
            &bbp,
            &config,
            &cwd,
            options.model.as_deref(),
        )?)
        .current_dir(cwd)
        .stdin(Stdio::null());
    for (name, _) in std::env::vars_os().filter(|(k, _)| k.to_string_lossy().starts_with("BBP_")) {
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
    command.env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
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
