#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `config` — resolve all settings at startup. Leaf crate (ARCHITECTURE §4).
//!
//! Settings resolve from three layers, highest precedence first (P1 foundation,
//! `docs/assessment/RECOMMENDATIONS.md`):
//!
//! 1. **Environment** (`RUSTYKEYS_*`) — the highest-precedence override.
//! 2. **Project file** — `<workspace>/.rustykeys/config.toml`, a `[settings]`
//!    table whose keys mirror the env vars (snake_case, no prefix). Other tables
//!    (e.g. `[mcp]`, read by `rk-mcp`) are ignored here.
//! 3. **Built-in defaults.**
//!
//! [`Config::resolve`] stays a pure function over an env getter (no I/O);
//! [`Config::from_env`] adds the one file read and merges the layers.

use std::path::{Path, PathBuf};

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

/// The `[settings]` table of `<workspace>/.rustykeys/config.toml` — the base
/// layer beneath the environment (P1). Every field is optional: an absent field
/// defers to the env var, then the built-in default. `workspace` is deliberately
/// absent (the file lives *inside* the workspace, so it cannot relocate it), as
/// are per-call provider settings the `config` crate does not own.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct Settings {
    model: Option<String>,
    harness_level: Option<HarnessLevel>,
    embed_model: Option<String>,
    allow_web: Option<bool>,
    permission_mode: Option<String>,
    allow_bypass: Option<bool>,
    allowed_tools: Option<Vec<String>>,
    isolation: Option<String>,
    context_limit: Option<usize>,
    compact_micro: Option<f64>,
    compact_session: Option<f64>,
    compact_full: Option<f64>,
    explore: Option<bool>,
    explore_branches: Option<usize>,
    explore_top_k: Option<usize>,
    otlp_endpoint: Option<String>,
    max_steps: Option<usize>,
}

/// The parsed project config file. Unknown tables (e.g. `[mcp]`, owned by
/// `rk-mcp`) are ignored so each crate parses only the section it owns.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct FileConfig {
    #[serde(default)]
    settings: Settings,
}

impl FileConfig {
    /// The project config path for a workspace.
    fn path_for(workspace: &Path) -> PathBuf {
        workspace.join(".rustykeys").join("config.toml")
    }

    /// Load `<workspace>/.rustykeys/config.toml`. A missing file yields the empty
    /// config (all settings deferred to env/defaults).
    fn load(workspace: &Path) -> Result<Self, ConfigError> {
        let path = Self::path_for(workspace);
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ConfigError::File(format!("{}: {e}", path.display()))),
        };
        toml::from_str(&body).map_err(|e| ConfigError::File(format!("{}: {e}", path.display())))
    }
}

impl Config {
    /// Resolve from the process environment plus the project config file
    /// (`<workspace>/.rustykeys/config.toml`). Env overrides file overrides
    /// defaults; `RUSTYKEYS_MODEL` is required (from either env or file).
    pub fn from_env() -> Result<Self, ConfigError> {
        let getter = |k: &str| std::env::var(k).ok();
        let workspace = Self::resolve_workspace(&getter)?;
        let file = FileConfig::load(&workspace)?;
        Self::resolve_with_file(getter, file.settings)
    }

    /// Resolve from an arbitrary env getter, with no project file (env layer
    /// only). Pure — touches neither the filesystem nor the process env — so it
    /// is the hermetic entry point for tests.
    pub fn resolve(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Self::resolve_with_file(get, Settings::default())
    }

    /// The workspace root: `RUSTYKEYS_WORKSPACE` if set, else the process cwd.
    /// Resolved before the project file is read (the file lives inside it) and is
    /// never overridable by the file.
    fn resolve_workspace(
        get: &impl Fn(&str) -> Option<String>,
    ) -> Result<PathBuf, ConfigError> {
        match get("RUSTYKEYS_WORKSPACE") {
            Some(p) if !p.trim().is_empty() => Ok(PathBuf::from(p)),
            _ => std::env::current_dir().map_err(|e| ConfigError::Workspace(e.to_string())),
        }
    }

    /// Resolve with the project file's `[settings]` as the base layer beneath the
    /// env getter (env > file > default). `RUSTYKEYS_MODEL` may come from either
    /// env or file; absent from both is an error.
    fn resolve_with_file(
        get: impl Fn(&str) -> Option<String>,
        file: Settings,
    ) -> Result<Self, ConfigError> {
        let model = get("RUSTYKEYS_MODEL")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| file.model.filter(|s| !s.trim().is_empty()))
            .ok_or(ConfigError::Missing {
                key: "RUSTYKEYS_MODEL",
            })?;

        let workspace = Self::resolve_workspace(&get)?;

        let harness_level = match get("RUSTYKEYS_HARNESS_LEVEL") {
            Some(s) if !s.trim().is_empty() => HarnessLevel::parse(&s)?,
            _ => file.harness_level.unwrap_or_default(),
        };

        let embed_model = get("RUSTYKEYS_EMBED_MODEL")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| file.embed_model.filter(|s| !s.trim().is_empty()));

        // Env truthiness wins when the var is set (non-empty); otherwise the file
        // value, otherwise the default `false`.
        let bool_or = |key: &str, file_val: Option<bool>| -> bool {
            match get(key) {
                Some(s) if !s.trim().is_empty() => {
                    matches!(s.trim(), "1" | "true" | "TRUE")
                }
                _ => file_val.unwrap_or(false),
            }
        };
        let allow_web = bool_or("RUSTYKEYS_ALLOW_WEB", file.allow_web);
        let allow_bypass = bool_or("RUSTYKEYS_ALLOW_BYPASS", file.allow_bypass);
        let permission_mode = get("RUSTYKEYS_PERMISSION_MODE")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| file.permission_mode.filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| "default".to_string());
        let allowed_tools = match get("RUSTYKEYS_ALLOWED_TOOLS") {
            Some(s) => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            None => file.allowed_tools.unwrap_or_default(),
        };
        let isolation = get("RUSTYKEYS_ISOLATION")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| file.isolation.filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| "none".to_string());

        let num = |key: &'static str, file_val: Option<f64>, default: f64| -> Result<f64, ConfigError> {
            match get(key) {
                Some(s) if !s.trim().is_empty() => s
                    .trim()
                    .parse()
                    .map_err(|_| ConfigError::Invalid { key, value: s }),
                _ => Ok(file_val.unwrap_or(default)),
            }
        };
        let usize_or =
            |key: &'static str, file_val: Option<usize>, default: usize| -> Result<usize, ConfigError> {
                match get(key) {
                    Some(s) if !s.trim().is_empty() => s
                        .trim()
                        .parse()
                        .map_err(|_| ConfigError::Invalid { key, value: s }),
                    _ => Ok(file_val.unwrap_or(default)),
                }
            };
        let context_limit = usize_or("RUSTYKEYS_CONTEXT_LIMIT", file.context_limit, 200_000)?;
        let compact_micro = num("RUSTYKEYS_COMPACT_MICRO", file.compact_micro, 0.80)?;
        let compact_session = num("RUSTYKEYS_COMPACT_SESSION", file.compact_session, 0.90)?;
        let compact_full = num("RUSTYKEYS_COMPACT_FULL", file.compact_full, 0.95)?;

        let explore = bool_or("RUSTYKEYS_EXPLORE", file.explore);
        let explore_branches = usize_or("RUSTYKEYS_EXPLORE_BRANCHES", file.explore_branches, 5)?;
        let explore_top_k = usize_or("RUSTYKEYS_EXPLORE_TOP_K", file.explore_top_k, 2)?;
        let max_steps = usize_or("RUSTYKEYS_MAX_STEPS", file.max_steps, 10)?;

        let otlp_endpoint = get("RUSTYKEYS_OTLP_ENDPOINT")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| file.otlp_endpoint.filter(|s| !s.trim().is_empty()));

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
    /// The project config file could not be read or parsed.
    #[error("cannot load project config: {0}")]
    File(String),
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

    /// Parse a `[settings]`/`[mcp]` TOML body the way `from_env` does.
    fn parse_file(body: &str) -> FileConfig {
        toml::from_str(body).expect("valid file config")
    }

    #[test]
    fn file_supplies_settings_when_env_absent() {
        let file = parse_file(
            r#"
            [settings]
            model = "ollama/llama3"
            harness_level = "h3"
            max_steps = 42
            isolation = "sandboxed"
            allow_web = true
            context_limit = 8000
            compact_micro = 0.5
            explore = true
            explore_branches = 3
            allowed_tools = ["read_file", "bash"]
        "#,
        );
        let cfg = Config::resolve_with_file(env(&[]), file.settings).unwrap();
        assert_eq!(cfg.model, "ollama/llama3");
        assert_eq!(cfg.harness_level, HarnessLevel::H3);
        assert_eq!(cfg.max_steps, 42);
        assert_eq!(cfg.isolation, "sandboxed");
        assert!(cfg.allow_web);
        assert_eq!(cfg.context_limit, 8000);
        assert_eq!(cfg.compact_micro, 0.5);
        assert!(cfg.explore);
        assert_eq!(cfg.explore_branches, 3);
        assert_eq!(cfg.allowed_tools, vec!["read_file", "bash"]);
        // Unset-in-file fields fall through to defaults.
        assert_eq!(cfg.explore_top_k, 2);
        assert_eq!(cfg.compact_full, 0.95);
    }

    #[test]
    fn env_overrides_file() {
        let file = parse_file(
            r#"
            [settings]
            model = "from-file"
            max_steps = 5
            isolation = "sandboxed"
            allow_web = true
        "#,
        );
        let cfg = Config::resolve_with_file(
            env(&[
                ("RUSTYKEYS_MODEL", "from-env"),
                ("RUSTYKEYS_MAX_STEPS", "99"),
                ("RUSTYKEYS_ISOLATION", "none"),
                ("RUSTYKEYS_ALLOW_WEB", "0"),
            ]),
            file.settings,
        )
        .unwrap();
        assert_eq!(cfg.model, "from-env");
        assert_eq!(cfg.max_steps, 99);
        assert_eq!(cfg.isolation, "none");
        assert!(!cfg.allow_web, "env '0' must override file 'true'");
    }

    #[test]
    fn unknown_tables_like_mcp_are_ignored_by_config() {
        // The unified file also carries `[mcp]` (owned by rk-mcp); the config
        // crate must parse only `[settings]` and ignore the rest.
        let file = parse_file(
            r#"
            [settings]
            model = "m"

            [[mcp.servers]]
            name = "filesystem"
            transport = "stdio"
            command = "npx"
        "#,
        );
        let cfg = Config::resolve_with_file(env(&[]), file.settings).unwrap();
        assert_eq!(cfg.model, "m");
    }

    #[test]
    fn env_invalid_value_rejected_even_with_file_present() {
        let file = parse_file("[settings]\nmodel = \"m\"\nmax_steps = 10\n");
        assert!(matches!(
            Config::resolve_with_file(
                env(&[("RUSTYKEYS_MAX_STEPS", "lots")]),
                file.settings
            ),
            Err(ConfigError::Invalid {
                key: "RUSTYKEYS_MAX_STEPS",
                ..
            })
        ));
    }

    #[test]
    fn missing_file_loads_as_empty_defaults() {
        let ws = std::env::temp_dir().join(format!("rk-cfgfile-none-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&ws);
        let file = FileConfig::load(&ws).unwrap();
        assert!(file.settings.model.is_none());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn load_reads_settings_from_workspace_file() {
        let ws = std::env::temp_dir().join(format!("rk-cfgfile-{}", std::process::id()));
        std::fs::create_dir_all(ws.join(".rustykeys")).unwrap();
        std::fs::write(
            ws.join(".rustykeys").join("config.toml"),
            "[settings]\nmodel = \"on-disk\"\nmax_steps = 7\n",
        )
        .unwrap();
        let file = FileConfig::load(&ws).unwrap();
        assert_eq!(file.settings.model.as_deref(), Some("on-disk"));
        assert_eq!(file.settings.max_steps, Some(7));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn malformed_file_is_a_file_error() {
        let ws = std::env::temp_dir().join(format!("rk-cfgfile-bad-{}", std::process::id()));
        std::fs::create_dir_all(ws.join(".rustykeys")).unwrap();
        std::fs::write(
            ws.join(".rustykeys").join("config.toml"),
            "this is = = not toml",
        )
        .unwrap();
        assert!(matches!(FileConfig::load(&ws), Err(ConfigError::File(_))));
        let _ = std::fs::remove_dir_all(&ws);
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
