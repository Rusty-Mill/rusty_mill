//! Policies vet a tool call before dispatch. `before_tool` is `async` from day
//! one so the Phase-7 ApprovalGate is not a breaking change (ADR-0016).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

/// Vets a tool call. `Ok(())` allows dispatch; `Err` blocks it.
#[async_trait]
pub trait Policy: Send + Sync {
    /// Inspect `(name, args)` before the tool body runs.
    async fn before_tool(&self, name: &str, args: &Value) -> Result<(), PolicyError>;
}

/// Policy veto (ADR-0023: one enum per library crate).
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// The call was blocked; the string is a model-facing reason.
    #[error("{0}")]
    Blocked(String),
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
        if matches!(name, "read_file" | "list_directory") {
            let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
            if !self.within_root(path) {
                return Err(PolicyError::Blocked(format!(
                    "path '{path}' is outside the workspace root"
                )));
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
        assert!(p.before_tool("read_file", &json!({"path": "src/a.rs"})).await.is_ok());
        assert!(p.before_tool("read_file", &json!({"path": "/ws/x"})).await.is_ok());
        assert!(p.before_tool("read_file", &json!({"path": "../etc"})).await.is_err());
        assert!(p.before_tool("read_file", &json!({"path": "/etc/passwd"})).await.is_err());
    }

    #[tokio::test]
    async fn chain_first_block_wins() {
        let chain = PolicyChain::new().with(Arc::new(WorkspacePolicy::new("/ws")));
        assert!(chain.before_tool("read_file", &json!({"path": "/etc/x"})).await.is_err());
        // A non-path tool is unconstrained by WorkspacePolicy.
        assert!(chain.before_tool("other", &json!({})).await.is_ok());
    }
}
