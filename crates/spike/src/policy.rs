//! The constrain seam: an `async before_tool` policy (ADR-0016) that vets every
//! tool call before dispatch (ADR-0007).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use crate::error::PolicyError;

/// Vets a tool call before it is dispatched. `before_tool` is `async` from day
/// one so the Phase-7 ApprovalGate is not a breaking change (ADR-0016).
#[async_trait]
pub trait Policy: Send + Sync {
    /// Inspect `(name, args)`. `Ok(())` allows dispatch; `Err` blocks it.
    async fn before_tool(&self, name: &str, args: &Value) -> Result<(), PolicyError>;
}

/// Confines filesystem tools to a workspace root. A `path` argument that
/// escapes the root is blocked.
pub struct WorkspacePolicy {
    root: PathBuf,
}

impl WorkspacePolicy {
    /// Build a policy rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Lexical containment check. The path is normalized against the root and
    /// must stay inside it. (`..` traversal is rejected without touching disk.)
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
        // Only path-bearing filesystem tools are constrained here.
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
