//! Provider-neutral pool and immutable per-task launch configuration.
use rusty_bbp::{ArtifactKind, ReadScope, Role, Roster};
use rusty_serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A pool member. Its launcher must speak BBP MCP; vendor names are opaque.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub vendor: String,
    pub program: String,
    pub args: Vec<String>,
    /// Run this member's harness confined (`bbp` launcher `sandbox`): it then
    /// reaches only these paths and variables, never the BBP directory.
    #[rusty_serde(default)]
    pub sandbox: Option<Sandbox>,
}
/// What a confined harness may reach beyond its own turn workspace: absolute
/// paths to read, absolute paths to write, and the names of the variables it
/// keeps. Passed to BBP unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sandbox {
    #[rusty_serde(default)]
    pub read: Vec<String>,
    #[rusty_serde(default)]
    pub write: Vec<String>,
    #[rusty_serde(default)]
    pub env: Vec<String>,
}
/// Explicit policy for one roster role. No role or provider is synthesized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub agent: String,
    pub builtin_tools: bool,
}
/// Goal's optional `bbp` object, or the file supplied by `--bbp`.
/// Paths are relative to the invoking directory. Workload profiles use the
/// existing BBP ProfileSet file format, including read roots, env and limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[rusty_serde(with = "path_json")]
    pub binary: PathBuf,
    #[rusty_serde(with = "path_json")]
    pub dir: PathBuf,
    #[rusty_serde(with = "path_json")]
    pub repo: PathBuf,
    #[rusty_serde(with = "path_json")]
    pub work: PathBuf,
    #[rusty_serde(with = "path_json")]
    pub profiles: PathBuf,
    pub pool: BTreeMap<String, Agent>,
    pub roster: Vec<Role>,
    pub assignments: BTreeMap<String, Assignment>,
    pub live_turns: u32,
    pub max_wall_secs: u64,
    /// Machine-wide admission shared by every moderator naming the same
    /// directory; without it each moderator launches as its turns allow.
    #[rusty_serde(default)]
    pub slots: Option<Slots>,
}
/// `bbp mod --slots`: how many harnesses (agents) and runners (validators)
/// may run at once across every task using `dir`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slots {
    #[rusty_serde(with = "path_json")]
    pub dir: PathBuf,
    pub agents: u32,
    pub validators: u32,
}
impl Config {
    /// Validate roster and independent-review policy before any host mutation.
    pub fn validate(&self) -> Result<(), String> {
        Roster::new(self.roster.clone()).map_err(|e| format!("roster: {e}"))?;
        if self.live_turns == 0
            || self.live_turns as usize > self.roster.len()
            || self.max_wall_secs == 0
        {
            return Err("live_turns must fit the roster; max_wall_secs must be positive".into());
        }
        if self
            .slots
            .as_ref()
            .is_some_and(|s| s.agents == 0 || s.validators == 0)
        {
            return Err("slots must admit at least one agent and one validator".into());
        }
        if self.assignments.len() != self.roster.len() {
            return Err("assign every roster role exactly once".into());
        }
        for role in &self.roster {
            let assignment = self
                .assignments
                .get(&role.id.0)
                .ok_or("missing role assignment")?;
            let agent = self
                .pool
                .get(&assignment.agent)
                .ok_or("assignment names absent pool member")?;
            if agent.vendor.trim().is_empty() || agent.program.trim().is_empty() {
                return Err("agent vendor and program must be nonblank".into());
            }
            if !role.final_decision {
                continue;
            }
            if role.read_messages != ReadScope::Scoped
                || !role.write.is_empty()
                || assignment.builtin_tools
                || role.read_artifacts.contains(&ArtifactKind::Log)
            {
                return Err(
                    "final reviewer must be scoped, read-only and have no built-in tools".into(),
                );
            }
            for writer in self
                .roster
                .iter()
                .filter(|r| r.write.contains(&ArtifactKind::Candidate))
            {
                let writer_assignment = self
                    .assignments
                    .get(&writer.id.0)
                    .ok_or("missing writer assignment")?;
                let writer_agent = self
                    .pool
                    .get(&writer_assignment.agent)
                    .ok_or("missing writer agent")?;
                if writer_assignment.agent == assignment.agent
                    || writer_agent.vendor == agent.vendor
                {
                    return Err(
                        "final review must use an independent pool member and vendor".into(),
                    );
                }
            }
        }
        Ok(())
    }
    /// Pin paths before a moderator detaches, so resume never depends on its cwd.
    pub fn absolute(mut self) -> Result<Self, String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let slots = self.slots.as_mut().map(|s| &mut s.dir);
        for p in [
            &mut self.dir,
            &mut self.repo,
            &mut self.work,
            &mut self.profiles,
        ]
        .into_iter()
        .chain(slots)
        {
            if p.is_relative() {
                *p = cwd.join(&*p);
            }
        }
        if self.binary.components().count() > 1 && self.binary.is_relative() {
            self.binary = cwd.join(&self.binary);
        }
        let repo = resolved(&self.repo)?;
        if resolved(&self.dir)?.starts_with(&repo) || resolved(&self.work)?.starts_with(&repo) {
            return Err("BBP state and runner work must be outside the source repository".into());
        }
        Ok(self)
    }
}

fn resolved(path: &std::path::Path) -> Result<PathBuf, String> {
    // Resolve existing ancestors, including symlinks, before comparing paths
    // that do not exist yet. Canonicalize also removes `..` components.
    if path.exists() {
        return path.canonicalize().map_err(|e| e.to_string());
    }
    let parent = path.parent().ok_or("host path has no existing ancestor")?;
    let base = resolved(parent)?;
    let name = path.file_name().ok_or("invalid host path")?;
    Ok(base.join(name))
}

/// Path encoding at the CLI boundary; refuse non-UTF8 paths rather than alias them.
pub(crate) mod path_json {
    use rusty_serde::Error;
    use rusty_serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::path::{Path, PathBuf};
    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        path.to_str()
            .ok_or_else(|| S::Error::custom("non-UTF8 host path"))?
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        Ok(PathBuf::from(String::deserialize(deserializer)?))
    }
}
