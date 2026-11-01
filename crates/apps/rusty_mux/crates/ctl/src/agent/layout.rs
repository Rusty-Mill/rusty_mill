//! Where each agent keeps the skill and the session-start hook, as pure path
//! arithmetic over the environment ([`AgentHomes`]) and the scope.
//!
//! | agent    | skill (user scope)                | hook (user scope)                      |
//! |----------|-----------------------------------|----------------------------------------|
//! | claude   | `~/.claude/skills/rusty-mux/`     | `SessionStart` in `~/.claude/settings.json` |
//! | codex    | `~/.agents/skills/rusty-mux/`     | `SessionStart` in `~/.codex/hooks.json` |
//! | opencode | `~/.agents/skills/rusty-mux/`     | plugin `~/.config/opencode/plugins/rusty-mux.js` |
//! | pi       | `~/.agents/skills/rusty-mux/`     | extension `~/.pi/agent/extensions/rusty-mux.ts` |
//!
//! Codex, OpenCode and Pi all read the shared Agent Skills directory
//! `.agents/skills`, so they share one skill file there. In project scope
//! the same paths sit under the project directory (`.claude`, `.agents`,
//! `.codex`, `.opencode/plugins`, `.pi/extensions`). `$CLAUDE_CONFIG_DIR`,
//! `$CODEX_HOME`, `$XDG_CONFIG_HOME` and `$PI_CODING_AGENT_DIR` move the
//! user-scope hook (and Claude Code's skill) as those agents do.

use std::path::{Path, PathBuf};

use super::scripts;
use crate::AgentHomes;
use crate::cli::{AgentKind, AgentScope};

/// What setup manages for one agent.
pub(super) struct Layout {
    pub agent: AgentKind,
    pub skill: PathBuf,
    pub hook: Hook,
}

/// The session-start hook, as the agent wants it.
pub(super) enum Hook {
    /// One command hook merged into a JSON settings file, whose original
    /// bytes setup keeps in `backup`.
    Settings {
        file: PathBuf,
        backup: PathBuf,
        command: String,
    },
    /// A whole file setup writes: a plugin or an extension.
    Script { file: PathBuf, text: String },
}

impl Hook {
    pub fn path(&self) -> &Path {
        match self {
            Self::Settings { file, .. } | Self::Script { file, .. } => file,
        }
    }
}

/// The layout of `agent` in `scope` for the ctl at `ctl`; `cwd` is the
/// project directory in project scope.
pub(super) fn layout(
    agent: AgentKind,
    scope: &AgentScope,
    homes: &AgentHomes,
    cwd: Option<&Path>,
    ctl: &str,
) -> Result<Layout, String> {
    let roots = Roots::new(scope, homes, cwd)?;
    let skill_base = match agent {
        AgentKind::Claude => roots.claude()?,
        AgentKind::Codex | AgentKind::OpenCode | AgentKind::Pi => roots.agents()?,
    };
    let skill = skill_base.join("skills").join("rusty-mux").join("SKILL.md");
    let hook = match agent {
        AgentKind::Claude => settings(roots.claude()?.join("settings.json"), ctl),
        AgentKind::Codex => settings(roots.codex()?.join("hooks.json"), ctl),
        AgentKind::OpenCode => Hook::Script {
            file: roots.opencode()?.join("rusty-mux.js"),
            text: scripts::opencode(ctl),
        },
        AgentKind::Pi => Hook::Script {
            file: roots.pi()?.join("rusty-mux.ts"),
            text: scripts::pi(ctl),
        },
    };
    Ok(Layout { agent, skill, hook })
}

fn settings(file: PathBuf, ctl: &str) -> Hook {
    let mut backup = file.clone().into_os_string();
    backup.push(".rusty-mux.bak");
    Hook::Settings {
        file,
        backup: backup.into(),
        command: super::settings::command(ctl),
    }
}

/// The directories a scope resolves to; each is asked for only by the
/// agents that need it, so a missing variable fails only those.
enum Roots<'a> {
    User(&'a AgentHomes),
    Project(&'a Path),
    Dir(&'a Path),
}

impl<'a> Roots<'a> {
    fn new(
        scope: &'a AgentScope,
        homes: &'a AgentHomes,
        cwd: Option<&'a Path>,
    ) -> Result<Self, String> {
        Ok(match scope {
            AgentScope::User => Self::User(homes),
            AgentScope::Project => {
                Self::Project(cwd.ok_or("--project: cannot tell the current directory")?)
            }
            AgentScope::Dir(dir) => Self::Dir(dir),
        })
    }

    fn claude(&self) -> Result<PathBuf, String> {
        match self {
            Self::Dir(dir) => Ok(dir.to_path_buf()),
            Self::Project(root) => Ok(root.join(".claude")),
            Self::User(homes) => own_or_home(
                &homes.claude,
                homes,
                &[".claude"],
                Some("CLAUDE_CONFIG_DIR"),
            ),
        }
    }

    fn agents(&self) -> Result<PathBuf, String> {
        match self {
            Self::Dir(_) => Err("--dir is only for Claude Code".into()),
            Self::Project(root) => Ok(root.join(".agents")),
            Self::User(homes) => own_or_home(&None, homes, &[".agents"], None),
        }
    }

    fn codex(&self) -> Result<PathBuf, String> {
        match self {
            Self::Dir(_) => Err("--dir is only for Claude Code".into()),
            Self::Project(root) => Ok(root.join(".codex")),
            Self::User(homes) => own_or_home(&homes.codex, homes, &[".codex"], Some("CODEX_HOME")),
        }
    }

    /// OpenCode's plugin directory.
    fn opencode(&self) -> Result<PathBuf, String> {
        match self {
            Self::Dir(_) => Err("--dir is only for Claude Code".into()),
            Self::Project(root) => Ok(root.join(".opencode").join("plugins")),
            Self::User(homes) => {
                Ok(
                    own_or_home(&homes.config, homes, &[".config"], Some("XDG_CONFIG_HOME"))?
                        .join("opencode")
                        .join("plugins"),
                )
            }
        }
    }

    /// Pi's extension directory.
    fn pi(&self) -> Result<PathBuf, String> {
        match self {
            Self::Dir(_) => Err("--dir is only for Claude Code".into()),
            Self::Project(root) => Ok(root.join(".pi").join("extensions")),
            Self::User(homes) => Ok(own_or_home(
                &homes.pi,
                homes,
                &[".pi", "agent"],
                Some("PI_CODING_AGENT_DIR"),
            )?
            .join("extensions")),
        }
    }
}

/// The agent's own variable, else `parts` under the home directory.
fn own_or_home(
    own: &Option<PathBuf>,
    homes: &AgentHomes,
    parts: &[&str],
    variable: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(dir) = own {
        return Ok(dir.clone());
    }
    let home = homes.home.as_ref().ok_or_else(|| match variable {
        Some(variable) => format!("no agent config directory: set {variable} or HOME"),
        None => "no home directory: set HOME".to_owned(),
    })?;
    Ok(parts.iter().fold(home.clone(), |dir, part| dir.join(part)))
}
