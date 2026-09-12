//! Forge-root path validation — re-exports from `nexus-types`.
//!
//! The validator itself lives in the leaf `nexus-types` crate so that
//! `nexus-kernel` and `nexus-plugins` (both of which need to call it on
//! the plugin write paths) can depend on it without creating a cycle
//! with `nexus-security`. This module re-exports the type and provides
//! a [`From`] conversion from [`PathValidationError`] to [`SecurityError`]
//! so that call sites inside `nexus-security` can continue to use the
//! unified security error surface.

pub use nexus_types::{ForgePathValidator, PathValidationError};

use crate::SecurityError;

impl From<PathValidationError> for SecurityError {
    fn from(err: PathValidationError) -> Self {
        match err {
            PathValidationError::PathTraversal(p) => SecurityError::PathTraversal(p),
            PathValidationError::InvalidPath(msg) => SecurityError::InvalidPath(msg),
        }
    }
}

/// Lexically normalize `path`: resolve `.` and `..` components without
/// touching the filesystem. Unlike [`std::fs::canonicalize`], this does not
/// require the path (or any of its ancestors) to exist, which is essential
/// for download destinations that name a file that does not exist yet.
///
/// A `..` component pops the preceding `Normal` component if present;
/// otherwise (at a filesystem root, or a relative path with no preceding
/// `Normal` component to pop) it is retained verbatim, matching the
/// behaviour of `path.Clean` in other ecosystems. Callers that need the
/// writable-root check in [`nexus_types::WritableRoot::is_path_writable`]
/// (documented as lexical-only, requiring the caller to normalize first)
/// should run untrusted destination paths through this before checking.
#[must_use]
pub(crate) fn normalize_lexical(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::{Component, PathBuf};

    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => match result.components().next_back() {
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) | None => {
                    // Can't go above a root, and nothing to pop on an empty
                    // relative path — retain the `..` verbatim.
                    result.push("..");
                }
                Some(Component::CurDir | Component::ParentDir) => {
                    result.push("..");
                }
            },
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn path_traversal_error_converts_to_security_error() {
        let err = PathValidationError::PathTraversal(PathBuf::from("/bad"));
        let sec: SecurityError = err.into();
        assert!(matches!(sec, SecurityError::PathTraversal(p) if p == Path::new("/bad")));
    }

    #[test]
    fn invalid_path_error_converts_to_security_error() {
        let err = PathValidationError::InvalidPath("null byte".to_string());
        let sec: SecurityError = err.into();
        assert!(matches!(sec, SecurityError::InvalidPath(msg) if msg.contains("null")));
    }
}
