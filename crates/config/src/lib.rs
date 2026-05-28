#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `config` — resolve all `RUSTYKEYS_*` settings at startup. Leaf crate
//! (ARCHITECTURE §4). No I/O beyond reading environment variables.

use std::path::PathBuf;

/// Maturity level of the harness (ARCHITECTURE §3). Phase 1 ships H1.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
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
    /// Kernel agent-loop step cap, from `RUSTYKEYS_MAX_STEPS` (default: 10).
    /// Wired into the aisdk loop via `stop_when` so a runaway tool-calling loop
    /// terminates; `compose::CleanTermination` then classifies the cap hit
    /// (ADR-0039, the P0 safety floor).
    pub max_steps: usize,
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
    /// Raw isolation profile, from `RUSTYKEYS_ISOLATION` (default `none`).
    /// Parsed by `feed::Isolation` (ADR-0030 / Phase 7B).
    pub isolation: String,
    /// Model context window in tokens, from `RUSTYKEYS_CONTEXT_LIMIT` (PRD 06).
    pub context_limit: usize,
    /// Micro-compaction threshold (fraction of `context_limit`), default 0.80.
    pub compact_micro: f64,
    /// Session-summary threshold, default 0.90.
    pub compact_session: f64,
    /// Full-compaction threshold, default 0.95.
    pub compact_full: f64,
    /// Whether the opt-in divergent→converge `explore` tool is enabled, from
    /// `RUSTYKEYS_EXPLORE` (`1`/`true`). Off by default — it is cost-gated
    /// (≈N+1 model calls per use; ADR-0032).
    pub explore: bool,
    /// Divergent branch count `N`, from `RUSTYKEYS_EXPLORE_BRANCHES` (default 5).
    pub explore_branches: usize,
    /// Converge top-`K`, from `RUSTYKEYS_EXPLORE_TOP_K` (default 2).
    pub explore_top_k: usize,
    /// OTLP collector endpoint for the pull-based exporter bound to the
    /// `KernelEvent` stream, from `RUSTYKEYS_OTLP_ENDPOINT` (ADR-0034). Absent ⇒
    /// the exporter is inert; stderr trace logging is unaffected.
    pub otlp_endpoint: Option<String>,
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
        let isolation = get("RUSTYKEYS_ISOLATION")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "none".to_string());

        let num = |key: &'static str, default: f64| -> Result<f64, ConfigError> {
            match get(key) {
                Some(s) if !s.trim().is_empty() => s
                    .trim()
                    .parse()
                    .map_err(|_| ConfigError::Invalid { key, value: s }),
                _ => Ok(default),
            }
        };
        let context_limit = match get("RUSTYKEYS_CONTEXT_LIMIT") {
            Some(s) if !s.trim().is_empty() => {
                s.trim().parse().map_err(|_| ConfigError::Invalid {
                    key: "RUSTYKEYS_CONTEXT_LIMIT",
                    value: s,
                })?
            }
            _ => 200_000,
        };
        let compact_micro = num("RUSTYKEYS_COMPACT_MICRO", 0.80)?;
        let compact_session = num("RUSTYKEYS_COMPACT_SESSION", 0.90)?;
        let compact_full = num("RUSTYKEYS_COMPACT_FULL", 0.95)?;

        let explore = truthy(get("RUSTYKEYS_EXPLORE"));
        let usize_or = |key: &'static str, default: usize| -> Result<usize, ConfigError> {
            match get(key) {
                Some(s) if !s.trim().is_empty() => s
                    .trim()
                    .parse()
                    .map_err(|_| ConfigError::Invalid { key, value: s }),
                _ => Ok(default),
            }
        };
        let explore_branches = usize_or("RUSTYKEYS_EXPLORE_BRANCHES", 5)?;
        let explore_top_k = usize_or("RUSTYKEYS_EXPLORE_TOP_K", 2)?;
        let max_steps = usize_or("RUSTYKEYS_MAX_STEPS", 10)?;

        let otlp_endpoint = get("RUSTYKEYS_OTLP_ENDPOINT").filter(|s| !s.trim().is_empty());

        Ok(Self {
            model,
            workspace,
            harness_level,
            max_steps,
            embed_model,
            allow_web,
            permission_mode,
            allow_bypass,
            allowed_tools,
            isolation,
            context_limit,
            compact_micro,
            compact_session,
            compact_full,
            explore,
            explore_branches,
            explore_top_k,
            otlp_endpoint,
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
        assert_eq!(cfg.isolation, "none"); // default
    }

    #[test]
    fn resolves_isolation_profile() {
        let cfg = Config::resolve(env(&[
            ("RUSTYKEYS_MODEL", "m"),
            ("RUSTYKEYS_ISOLATION", "sandboxed"),
        ]))
        .unwrap();
        assert_eq!(cfg.isolation, "sandboxed");
    }

    #[test]
    fn otlp_endpoint_absent_by_default_and_parsed_when_set() {
        let def = Config::resolve(env(&[("RUSTYKEYS_MODEL", "m")])).unwrap();
        assert_eq!(def.otlp_endpoint, None);

        let cfg = Config::resolve(env(&[
            ("RUSTYKEYS_MODEL", "m"),
            ("RUSTYKEYS_OTLP_ENDPOINT", "http://localhost:4317"),
        ]))
        .unwrap();
        assert_eq!(cfg.otlp_endpoint.as_deref(), Some("http://localhost:4317"));

        // Whitespace-only ⇒ treated as absent.
        let blank = Config::resolve(env(&[
            ("RUSTYKEYS_MODEL", "m"),
            ("RUSTYKEYS_OTLP_ENDPOINT", "  "),
        ]))
        .unwrap();
        assert_eq!(blank.otlp_endpoint, None);
    }

    #[test]
    fn token_budget_defaults_and_overrides() {
        let def = Config::resolve(env(&[("RUSTYKEYS_MODEL", "m")])).unwrap();
        assert_eq!(def.context_limit, 200_000);
        assert_eq!(def.compact_micro, 0.80);
        assert_eq!(def.compact_full, 0.95);

        let cfg = Config::resolve(env(&[
            ("RUSTYKEYS_MODEL", "m"),
            ("RUSTYKEYS_CONTEXT_LIMIT", "8000"),
            ("RUSTYKEYS_COMPACT_MICRO", "0.5"),
        ]))
        .unwrap();
        assert_eq!(cfg.context_limit, 8000);
        assert_eq!(cfg.compact_micro, 0.5);
    }

    #[test]
    fn explore_is_off_by_default_and_opt_in() {
        let def = Config::resolve(env(&[("RUSTYKEYS_MODEL", "m")])).unwrap();
        assert!(!def.explore);
        assert_eq!(def.explore_branches, 5);
        assert_eq!(def.explore_top_k, 2);

        let on = Config::resolve(env(&[
            ("RUSTYKEYS_MODEL", "m"),
            ("RUSTYKEYS_EXPLORE", "1"),
            ("RUSTYKEYS_EXPLORE_BRANCHES", "3"),
            ("RUSTYKEYS_EXPLORE_TOP_K", "1"),
        ]))
        .unwrap();
        assert!(on.explore);
        assert_eq!(on.explore_branches, 3);
        assert_eq!(on.explore_top_k, 1);
    }

    #[test]
    fn max_steps_defaults_and_overrides() {
        let def = Config::resolve(env(&[("RUSTYKEYS_MODEL", "m")])).unwrap();
        assert_eq!(def.max_steps, 10);

        let cfg = Config::resolve(env(&[
            ("RUSTYKEYS_MODEL", "m"),
            ("RUSTYKEYS_MAX_STEPS", "25"),
        ]))
        .unwrap();
        assert_eq!(cfg.max_steps, 25);
    }

    #[test]
    fn invalid_max_steps_is_rejected() {
        assert!(matches!(
            Config::resolve(env(&[
                ("RUSTYKEYS_MODEL", "m"),
                ("RUSTYKEYS_MAX_STEPS", "many"),
            ])),
            Err(ConfigError::Invalid {
                key: "RUSTYKEYS_MAX_STEPS",
                ..
            })
        ));
    }

    #[test]
    fn invalid_context_limit_is_rejected() {
        assert!(matches!(
            Config::resolve(env(&[
                ("RUSTYKEYS_MODEL", "m"),
                ("RUSTYKEYS_CONTEXT_LIMIT", "lots"),
            ])),
            Err(ConfigError::Invalid {
                key: "RUSTYKEYS_CONTEXT_LIMIT",
                ..
            })
        ));
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
