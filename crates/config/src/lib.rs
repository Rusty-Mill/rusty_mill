#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `config` — resolve all `RUSTYKEYS_*` settings at startup. Leaf crate
//! (ARCHITECTURE §4). No I/O beyond reading environment variables.

use std::path::PathBuf;

/// Maturity level of the harness (ARCHITECTURE §3). Phase 1 ships H1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum HarnessLevel {
    /// Task + repo files, no tool registry.
    H0,
    /// Tool registry + tool-use protocol. The Phase-1 default.
    #[default]
    H1,
    /// Project memory, Task State, context selection.
    H2,
    /// Deterministic checks, attribution, verification.
    H3,
}

impl HarnessLevel {
    fn parse(s: &str) -> Result<Self, ConfigError> {
        match s.to_ascii_lowercase().as_str() {
            "h0" => Ok(Self::H0),
            "h1" => Ok(Self::H1),
            "h2" => Ok(Self::H2),
            "h3" => Ok(Self::H3),
            other => Err(ConfigError::Invalid {
                key: "RUSTYKEYS_HARNESS_LEVEL",
                value: other.to_string(),
            }),
        }
    }
}

/// Resolved configuration for a session.
#[derive(Debug, Clone)]
pub struct Config {
    /// aisdk model string (any provider), from `RUSTYKEYS_MODEL`.
    pub model: String,
    /// Workspace root and policy boundary, from `RUSTYKEYS_WORKSPACE` (default: cwd).
    pub workspace: PathBuf,
    /// Harness maturity level, from `RUSTYKEYS_HARNESS_LEVEL` (default: H1).
    pub harness_level: HarnessLevel,
    /// Optional embedding model, from `RUSTYKEYS_EMBED_MODEL`. Set ⇒ semantic
    /// recall; unset ⇒ lexical fallback (PRD 03 / Phase 5).
    pub embed_model: Option<String>,
    /// Whether web tools are enabled, from `RUSTYKEYS_ALLOW_WEB` (`1`/`true`).
    /// Off by default (PRD 03; the SSRF guard still applies when on).
    pub allow_web: bool,
    /// Raw permission mode, from `RUSTYKEYS_PERMISSION_MODE` (default `default`).
    /// Parsed by `constrain::PermissionMode` (PRD 02).
    pub permission_mode: String,
    /// Whether `bypass` mode is permitted, from `RUSTYKEYS_ALLOW_BYPASS` (`1`).
    pub allow_bypass: bool,
    /// Allowed tools for `restricted` mode, from `RUSTYKEYS_ALLOWED_TOOLS` (CSV).
    pub allowed_tools: Vec<String>,
}

impl Config {
    /// Resolve from the process environment. `RUSTYKEYS_MODEL` is required.
    pub fn from_env() -> Result<Self, ConfigError> {
        let getter = |k: &str| std::env::var(k).ok();
        Self::resolve(getter)
    }

    /// Resolve from an arbitrary getter (testable without touching the process env).
    pub fn resolve(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let model = get("RUSTYKEYS_MODEL")
            .filter(|s| !s.trim().is_empty())
            .ok_or(ConfigError::Missing {
                key: "RUSTYKEYS_MODEL",
            })?;

        let workspace = match get("RUSTYKEYS_WORKSPACE") {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => std::env::current_dir().map_err(|e| ConfigError::Workspace(e.to_string()))?,
        };

        let harness_level = match get("RUSTYKEYS_HARNESS_LEVEL") {
            Some(s) if !s.trim().is_empty() => HarnessLevel::parse(&s)?,
            _ => HarnessLevel::default(),
        };

        let embed_model = get("RUSTYKEYS_EMBED_MODEL").filter(|s| !s.trim().is_empty());

        let truthy =
            |v: Option<String>| matches!(v.as_deref(), Some("1") | Some("true") | Some("TRUE"));
        let allow_web = truthy(get("RUSTYKEYS_ALLOW_WEB"));
        let allow_bypass = truthy(get("RUSTYKEYS_ALLOW_BYPASS"));
        let permission_mode = get("RUSTYKEYS_PERMISSION_MODE")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "default".to_string());
        let allowed_tools = get("RUSTYKEYS_ALLOWED_TOOLS")
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            model,
            workspace,
            harness_level,
            embed_model,
            allow_web,
            permission_mode,
            allow_bypass,
            allowed_tools,
        })
    }
}

/// Configuration resolution errors (ADR-0023: one enum per library crate).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required setting was absent or empty.
    #[error("required setting {key} is not set")]
    Missing {
        /// The env var name.
        key: &'static str,
    },
    /// A setting had an unparseable value.
    #[error("invalid value '{value}' for {key}")]
    Invalid {
        /// The env var name.
        key: &'static str,
        /// The offending value.
        value: String,
    },
    /// The workspace directory could not be resolved.
    #[error("cannot resolve workspace: {0}")]
    Workspace(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn missing_model_is_an_error() {
        assert!(matches!(
            Config::resolve(env(&[])),
            Err(ConfigError::Missing {
                key: "RUSTYKEYS_MODEL"
            })
        ));
    }

    #[test]
    fn resolves_model_workspace_level() {
        let cfg = Config::resolve(env(&[
            ("RUSTYKEYS_MODEL", "ollama/llama3"),
            ("RUSTYKEYS_WORKSPACE", "/tmp/ws"),
            ("RUSTYKEYS_HARNESS_LEVEL", "H3"),
        ]))
        .unwrap();
        assert_eq!(cfg.model, "ollama/llama3");
        assert_eq!(cfg.workspace, PathBuf::from("/tmp/ws"));
        assert_eq!(cfg.harness_level, HarnessLevel::H3);
    }

    #[test]
    fn invalid_level_is_rejected() {
        assert!(matches!(
            Config::resolve(env(&[
                ("RUSTYKEYS_MODEL", "m"),
                ("RUSTYKEYS_HARNESS_LEVEL", "h9"),
            ])),
            Err(ConfigError::Invalid {
                key: "RUSTYKEYS_HARNESS_LEVEL",
                ..
            })
        ));
    }
}
