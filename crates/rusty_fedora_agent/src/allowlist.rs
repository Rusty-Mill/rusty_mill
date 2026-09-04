//! The allowlists a human edits to expand this agent's scope: which
//! systemd units it may control, which dnf packages it may install/
//! remove, and which config-file path prefixes it may read/write. Loaded
//! once from a TOML file at startup -- explicit configuration, no magic
//! globals, and the thing a human edits to expand scope later (see
//! `deploy/allowlist.toml`).

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::error::AgentError;

/// The on-disk shape of the allowlist config file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AllowlistConfig {
    /// Exact systemd unit names (e.g. `ollama.service`) this agent may
    /// start/stop/restart/enable/disable.
    #[serde(default)]
    pub units: Vec<String>,
    /// Exact dnf package names this agent may install/remove.
    #[serde(default)]
    pub packages: Vec<String>,
    /// Absolute path prefixes this agent may read/write config under.
    #[serde(default)]
    pub config_path_prefixes: Vec<PathBuf>,
}

impl AllowlistConfig {
    /// Load and parse the allowlist config file at `path`.
    pub fn load(path: &Path) -> Result<Self, AgentError> {
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|e| {
            AgentError::InvalidRequest(format!(
                "malformed allowlist config at {}: {e}",
                path.display()
            ))
        })
    }
}

/// Scope checks derived from an [`AllowlistConfig`]. Every mutating or
/// filesystem-touching operation in [`crate::systemd`]/[`crate::dnf`]/
/// [`crate::config_files`] calls one of these *before* building a
/// `Command` or touching a path -- an illegal unit/package/path never
/// reaches `exec` or the filesystem.
#[derive(Debug, Clone)]
pub struct Allowlist {
    config: AllowlistConfig,
}

impl Allowlist {
    pub fn new(config: AllowlistConfig) -> Self {
        Self { config }
    }

    /// Is `name` an exact match in the unit allowlist?
    pub fn check_unit(&self, name: &str) -> Result<(), AgentError> {
        if self.config.units.iter().any(|u| u == name) {
            Ok(())
        } else {
            Err(AgentError::UnitNotAllowed(name.to_string()))
        }
    }

    /// Is `name` a syntactically plausible package name *and* an exact
    /// match in the package allowlist? The syntax check runs first and
    /// independently of the allowlist -- defense in depth, since `dnf` is
    /// invoked with `name` as a bare argv element (never through a shell),
    /// so this is not an injection boundary, but a name that isn't a valid
    /// dnf package identifier is never legitimate input either way.
    pub fn check_package(&self, name: &str) -> Result<(), AgentError> {
        let valid_syntax = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | ':'));
        if !valid_syntax {
            return Err(AgentError::InvalidRequest(format!(
                "'{name}' is not a valid package name"
            )));
        }
        if self.config.packages.iter().any(|p| p == name) {
            Ok(())
        } else {
            Err(AgentError::PackageNotAllowed(name.to_string()))
        }
    }

    /// Is `path` absolute, free of `..` traversal, and under one of the
    /// configured prefixes? Returns the (already-validated) path back so
    /// callers don't re-derive it.
    pub fn check_config_path(&self, path: &Path) -> Result<PathBuf, AgentError> {
        if !path.is_absolute() {
            return Err(AgentError::PathNotAllowed(path.display().to_string()));
        }
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(AgentError::PathNotAllowed(path.display().to_string()));
        }
        if self
            .config
            .config_path_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix))
        {
            Ok(path.to_path_buf())
        } else {
            Err(AgentError::PathNotAllowed(path.display().to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowlist() -> Allowlist {
        Allowlist::new(AllowlistConfig {
            units: vec!["ollama.service".to_string()],
            packages: vec!["htop".to_string()],
            config_path_prefixes: vec![PathBuf::from("/etc/systemd")],
        })
    }

    #[test]
    fn allowed_unit_passes() {
        assert!(allowlist().check_unit("ollama.service").is_ok());
    }

    #[test]
    fn unlisted_unit_is_rejected() {
        let err = allowlist()
            .check_unit("sshd.service")
            .expect_err("sshd.service is not allowlisted");
        assert!(matches!(err, AgentError::UnitNotAllowed(name) if name == "sshd.service"));
    }

    #[test]
    fn allowed_package_passes() {
        assert!(allowlist().check_package("htop").is_ok());
    }

    #[test]
    fn unlisted_package_is_rejected() {
        let err = allowlist()
            .check_package("nmap")
            .expect_err("nmap is not allowlisted");
        assert!(matches!(err, AgentError::PackageNotAllowed(name) if name == "nmap"));
    }

    #[test]
    fn a_package_name_with_shell_metacharacters_is_rejected_before_the_allowlist_check() {
        // Not itself an injection risk (dnf is invoked without a shell),
        // but it can never be legitimate input, and this path proves it
        // never gets far enough to be looked up in the allowlist at all.
        let err = allowlist()
            .check_package("htop; rm -rf /")
            .expect_err("not a valid package name");
        assert!(matches!(err, AgentError::InvalidRequest(_)));
    }

    #[test]
    fn path_under_an_allowed_prefix_passes() {
        assert!(
            allowlist()
                .check_config_path(Path::new(
                    "/etc/systemd/system/ollama.service.d/override.conf"
                ))
                .is_ok()
        );
    }

    #[test]
    fn path_outside_every_prefix_is_rejected() {
        let err = allowlist()
            .check_config_path(Path::new("/etc/shadow"))
            .expect_err("outside every allowed prefix");
        assert!(matches!(err, AgentError::PathNotAllowed(_)));
    }

    #[test]
    fn path_traversal_out_of_an_allowed_prefix_is_rejected() {
        let err = allowlist()
            .check_config_path(Path::new("/etc/systemd/../shadow"))
            .expect_err("traversal out of the allowed prefix");
        assert!(matches!(err, AgentError::PathNotAllowed(_)));
    }

    #[test]
    fn a_relative_path_is_rejected() {
        let err = allowlist()
            .check_config_path(Path::new("etc/systemd/system/foo.conf"))
            .expect_err("a relative path is not allowed");
        assert!(matches!(err, AgentError::PathNotAllowed(_)));
    }

    #[test]
    fn an_empty_allowlist_config_parses_and_permits_nothing() {
        let config: AllowlistConfig = toml::from_str("").expect("empty config is valid");
        let allowlist = Allowlist::new(config);
        assert!(allowlist.check_unit("anything.service").is_err());
        assert!(allowlist.check_package("anything").is_err());
        assert!(
            allowlist
                .check_config_path(Path::new("/etc/anything"))
                .is_err()
        );
    }
}
