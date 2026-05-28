//! Policies vet a tool call before dispatch. `before_tool` is `async` from day
//! one so the Phase-7 ApprovalGate is not a breaking change (ADR-0016).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::security::{default_checkers, SecurityCheck, SecurityLog};

/// Vets a tool call. `Ok(())` allows dispatch; `Err` blocks it.
#[async_trait]
pub trait Policy: Send + Sync {
    /// Inspect `(name, args)` before the tool body runs.
    async fn before_tool(&self, name: &str, args: &Value) -> Result<(), PolicyError>;
}

/// Policy veto (ADR-0023; error-handling §3). Structured so the compose layer
/// and `security.jsonl` derive the block reason from the *variant*, never by
/// parsing prose. Additional variants (mode gates, the approval gate) land with
/// Phase 7.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// A path argument escaped the workspace root.
    #[error("path {0} escapes the workspace root")]
    OutsideWorkspace(PathBuf),
    /// The active permission mode forbids this tool.
    #[error("mode {mode} forbids tool {tool}")]
    ModeForbidden {
        /// The active mode (snake_case).
        mode: &'static str,
        /// The forbidden tool name.
        tool: String,
    },
    /// A security checker blocked the call (`bash`). The `checker` variant name
    /// is the structured `security.jsonl` field (ADR-0023).
    #[error("blocked by {checker}: matched '{pattern}'")]
    SecurityCheck {
        /// The checker that blocked (e.g. `CommandInjectionCheck`).
        checker: &'static str,
        /// The matched pattern.
        pattern: String,
    },
    /// The human (or remote ACL) denied an approval request (ApprovalGate).
    #[error("blocked by approval gate")]
    ApprovalDenied,
}

/// Runs an ordered set of policies; the first block wins (fail-closed).
#[derive(Default, Clone)]
pub struct PolicyChain {
    policies: Vec<Arc<dyn Policy>>,
}

impl PolicyChain {
    /// Empty chain (allows everything).
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a policy.
    pub fn with(mut self, policy: Arc<dyn Policy>) -> Self {
        self.policies.push(policy);
        self
    }
}

#[async_trait]
impl Policy for PolicyChain {
    async fn before_tool(&self, name: &str, args: &Value) -> Result<(), PolicyError> {
        for policy in &self.policies {
            policy.before_tool(name, args).await?;
        }
        Ok(())
    }
}

/// Confines path-bearing filesystem tools to a workspace root.
pub struct WorkspacePolicy {
    root: PathBuf,
}

impl WorkspacePolicy {
    /// Build a policy rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Lexical containment: normalize `candidate` against the root (rejecting
    /// `..` escapes) without touching disk.
    fn within_root(&self, candidate: &str) -> bool {
        let joined = if Path::new(candidate).is_absolute() {
            PathBuf::from(candidate)
        } else {
            self.root.join(candidate)
        };
        let mut normalized = PathBuf::new();
        for component in joined.components() {
            use std::path::Component::*;
            match component {
                ParentDir => {
                    if !normalized.pop() {
                        return false;
                    }
                }
                CurDir => {}
                other => normalized.push(other.as_os_str()),
            }
        }
        normalized.starts_with(&self.root)
    }
}

#[async_trait]
impl Policy for WorkspacePolicy {
    async fn before_tool(&self, name: &str, args: &Value) -> Result<(), PolicyError> {
        // Path-bearing tools are confined to the workspace. `glob`'s pattern and
        // `grep`'s optional path are checked under the `pattern`/`path` keys.
        let candidate = match name {
            "read_file" | "list_directory" | "write_file" | "edit_file" => {
                args.get("path").and_then(Value::as_str)
            }
            "glob" => args.get("pattern").and_then(Value::as_str),
            "grep" => args.get("path").and_then(Value::as_str),
            _ => None,
        };
        if let Some(path) = candidate {
            if !self.within_root(path) {
                return Err(PolicyError::OutsideWorkspace(PathBuf::from(path)));
            }
        }
        Ok(())
    }
}

/// Named permission modes gating tool classes (PRD 02). The enum lives here
/// (not `config`) so the DAG stays `constrain → config`; `Config` carries the
/// raw strings and [`PermissionMode::from_config`] parses them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionMode {
    /// Workspace boundary + security checks; everything else allowed.
    Default,
    /// Read-only + propose: no writes, no bash.
    Plan,
    /// Writes allowed without an approval prompt; bash still checked.
    AcceptEdits,
    /// No writes, no bash — safe exploration.
    ReadOnly,
    /// Only the explicit allowlist.
    Restricted(Vec<String>),
    /// All policy checks disabled — requires `RUSTYKEYS_ALLOW_BYPASS=1`.
    Bypass,
}

fn is_write_tool(t: &str) -> bool {
    matches!(t, "write_file" | "edit_file")
}

fn is_exec_tool(t: &str) -> bool {
    t == "bash"
}

impl PermissionMode {
    /// Parse from config. `bypass` without `allow_bypass` is refused (downgraded
    /// to `Default` with a stderr warning) — bypass requires the explicit flag.
    pub fn from_config(mode: &str, allow_bypass: bool, allowed_tools: &[String]) -> Self {
        match mode
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "_")
            .as_str()
        {
            "plan" => PermissionMode::Plan,
            "accept_edits" => PermissionMode::AcceptEdits,
            "read_only" => PermissionMode::ReadOnly,
            "restricted" => PermissionMode::Restricted(allowed_tools.to_vec()),
            "bypass" if allow_bypass => PermissionMode::Bypass,
            "bypass" => {
                eprintln!(
                    "rusty-keys: RUSTYKEYS_PERMISSION_MODE=bypass ignored — set RUSTYKEYS_ALLOW_BYPASS=1 to enable; using default"
                );
                PermissionMode::Default
            }
            _ => PermissionMode::Default,
        }
    }

    /// snake_case label.
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::Plan => "plan",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::ReadOnly => "read_only",
            PermissionMode::Restricted(_) => "restricted",
            PermissionMode::Bypass => "bypass",
        }
    }

    /// Is `tool` permitted under this mode?
    pub fn check(&self, tool: &str) -> Result<(), PolicyError> {
        let forbid = || {
            Err(PolicyError::ModeForbidden {
                mode: self.as_str(),
                tool: tool.to_string(),
            })
        };
        match self {
            PermissionMode::Default | PermissionMode::AcceptEdits | PermissionMode::Bypass => {
                Ok(())
            }
            PermissionMode::Plan | PermissionMode::ReadOnly => {
                if is_write_tool(tool) || is_exec_tool(tool) {
                    forbid()
                } else {
                    Ok(())
                }
            }
            PermissionMode::Restricted(allowed) => {
                if allowed.iter().any(|t| t == tool) {
                    Ok(())
                } else {
                    forbid()
                }
            }
        }
    }
}

/// A policy that gates tools by the active [`PermissionMode`] (PRD 02). The mode
/// is read live from a shared [`PlanController`] so plan mode (PRD 06) can flip
/// it at runtime.
pub struct ModePolicy {
    controller: Arc<crate::PlanController>,
}

impl ModePolicy {
    /// Gate tools by a fixed `mode` (its own controller; no runtime transitions).
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            controller: Arc::new(crate::PlanController::new(mode)),
        }
    }

    /// Gate tools by a shared [`PlanController`] (lets plan-mode tools and the
    /// session drive transitions).
    pub fn shared(controller: Arc<crate::PlanController>) -> Self {
        Self { controller }
    }
}

#[async_trait]
impl Policy for ModePolicy {
    async fn before_tool(&self, name: &str, _args: &Value) -> Result<(), PolicyError> {
        self.controller.mode().check(name)
    }
}

/// Runs the [`SecurityCheck`] suite over `bash` commands (PRD 02). A deny-list,
/// not a sandbox — the OS-level `ToolExecutor` isolation is Phase 7B (ADR-0030).
/// A block writes a redacted `SecurityEvent` to `security.jsonl` when a log is
/// attached.
pub struct BashGuard {
    checkers: Vec<Box<dyn SecurityCheck>>,
    log: Option<Arc<SecurityLog>>,
}

impl Default for BashGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl BashGuard {
    /// Build with the default checker suite and no security log.
    pub fn new() -> Self {
        Self {
            checkers: default_checkers(),
            log: None,
        }
    }

    /// Attach an append-only security log; blocked calls are recorded to it.
    pub fn with_log(mut self, log: Arc<SecurityLog>) -> Self {
        self.log = Some(log);
        self
    }
}

#[async_trait]
impl Policy for BashGuard {
    async fn before_tool(&self, name: &str, args: &Value) -> Result<(), PolicyError> {
        if name != "bash" {
            return Ok(());
        }
        let command = args.get("command").and_then(Value::as_str).unwrap_or("");
        for checker in &self.checkers {
            if let Err(e) = checker.check(command) {
                if let (Some(log), PolicyError::SecurityCheck { checker, pattern }) =
                    (&self.log, &e)
                {
                    log.record(name, checker, pattern, args);
                }
                return Err(e);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn blocks_escape_allows_inside() {
        let p = WorkspacePolicy::new("/ws");
        assert!(p
            .before_tool("read_file", &json!({"path": "src/a.rs"}))
            .await
            .is_ok());
        assert!(p
            .before_tool("read_file", &json!({"path": "/ws/x"}))
            .await
            .is_ok());
        assert!(p
            .before_tool("read_file", &json!({"path": "../etc"}))
            .await
            .is_err());
        assert!(p
            .before_tool("read_file", &json!({"path": "/etc/passwd"}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn bashguard_blocks_destructive_allows_safe() {
        let g = BashGuard::new();
        assert!(g
            .before_tool("bash", &json!({"command": "rm -rf / --no-preserve-root"}))
            .await
            .is_err());
        assert!(g
            .before_tool("bash", &json!({"command": "RM  -RF  /"}))
            .await
            .is_err()); // normalized
        assert!(g
            .before_tool("bash", &json!({"command": "cargo test"}))
            .await
            .is_ok());
        // Non-bash tools are unaffected.
        assert!(g
            .before_tool("read_file", &json!({"path": "rm -rf /"}))
            .await
            .is_ok());
    }

    #[test]
    fn mode_gates_writes_and_exec() {
        let ro = PermissionMode::ReadOnly;
        assert!(ro.check("read_file").is_ok());
        assert!(ro.check("write_file").is_err());
        assert!(ro.check("edit_file").is_err());
        assert!(ro.check("bash").is_err());

        let plan = PermissionMode::Plan;
        assert!(plan.check("grep").is_ok());
        assert!(plan.check("write_file").is_err());

        let edits = PermissionMode::AcceptEdits;
        assert!(edits.check("write_file").is_ok());
        assert!(edits.check("bash").is_ok());

        let restricted = PermissionMode::Restricted(vec!["read_file".into()]);
        assert!(restricted.check("read_file").is_ok());
        assert!(restricted.check("grep").is_err());
    }

    #[test]
    fn bypass_requires_explicit_flag() {
        // Without the flag, bypass downgrades to Default.
        assert_eq!(
            PermissionMode::from_config("bypass", false, &[]),
            PermissionMode::Default
        );
        assert_eq!(
            PermissionMode::from_config("bypass", true, &[]),
            PermissionMode::Bypass
        );
        // Hyphen/case normalization.
        assert_eq!(
            PermissionMode::from_config("Accept-Edits", false, &[]),
            PermissionMode::AcceptEdits
        );
    }

    #[tokio::test]
    async fn chain_first_block_wins() {
        let chain = PolicyChain::new().with(Arc::new(WorkspacePolicy::new("/ws")));
        assert!(chain
            .before_tool("read_file", &json!({"path": "/etc/x"}))
            .await
            .is_err());
        // A non-path tool is unconstrained by WorkspacePolicy.
        assert!(chain.before_tool("other", &json!({})).await.is_ok());
    }
}
