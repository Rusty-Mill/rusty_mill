//! The deterministic check registry (PRD 05 / Phase 10). Project- and
//! local-level `checks.toml` define shell checks the agent (H3) and — later, the
//! evaluator (R5) — run to verify a turn against requirement ids. A check passes
//! when its command output contains `expected_substring`. Each result projects
//! into one `verification_trace` entry (the `registered_test` / `targeted_test`
//! / `full_regression` / `lint` methods).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::ComposeError;

/// One registered deterministic check (a `[[check]]` entry in `checks.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct DeterministicCheck {
    /// Stable check name (the registry key; local overrides project by name).
    pub name: String,
    /// Shell command to execute (run via `sh -c` in the workspace).
    pub command: String,
    /// Output must contain this substring for the check to pass.
    #[serde(default)]
    pub expected_substring: String,
    /// Requirement ids this check covers (the `verification_trace` `covers[]`).
    #[serde(default)]
    pub covers: Vec<String>,
    /// Controlled-vocabulary method; defaults to `registered_test`.
    #[serde(default = "default_method")]
    pub method: String,
    /// Per-check timeout in milliseconds (default 60s).
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_method() -> String {
    "registered_test".to_string()
}

fn default_timeout() -> u64 {
    60_000
}

#[derive(Debug, Deserialize)]
struct ChecksFile {
    #[serde(default, rename = "check")]
    checks: Vec<DeterministicCheck>,
}

/// One check's run result, carrying the requirement coverage + method so the
/// assembler can project a `verification_trace` entry.
#[derive(Debug, Clone)]
pub struct CheckRunResult {
    /// The check name.
    pub check: String,
    /// The method (`registered_test` / `targeted_test` / `full_regression` / `lint`).
    pub method: String,
    /// Observed command output (combined stdout+stderr, trimmed).
    pub observed: String,
    /// The expected substring.
    pub expected: String,
    /// Requirement ids covered.
    pub covers: Vec<String>,
    /// Whether the check passed.
    pub passed: bool,
    /// Wall-clock duration.
    pub duration_ms: u64,
}

/// A set of deterministic checks, rooted at a workspace, runnable as a batch.
pub struct CheckRegistry {
    checks: Vec<DeterministicCheck>,
    workspace: PathBuf,
}

impl CheckRegistry {
    /// Load checks for `workspace`, merging the project file
    /// (`harness/checks.toml`) with the local file (`.rustykeys/checks.toml`);
    /// the local file takes precedence per check name. A missing file is empty.
    pub fn load(workspace: &Path) -> Result<Self, ComposeError> {
        let mut by_name: BTreeMap<String, DeterministicCheck> = BTreeMap::new();
        for rel in ["harness/checks.toml", ".rustykeys/checks.toml"] {
            for c in Self::read_file(&workspace.join(rel))? {
                by_name.insert(c.name.clone(), c);
            }
        }
        Ok(Self {
            checks: by_name.into_values().collect(),
            workspace: workspace.to_path_buf(),
        })
    }

    fn read_file(path: &Path) -> Result<Vec<DeterministicCheck>, ComposeError> {
        let body = match std::fs::read_to_string(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let parsed: ChecksFile =
            toml::from_str(&body).map_err(|e| ComposeError::Checks(e.to_string()))?;
        Ok(parsed.checks)
    }

    /// Whether any checks are registered.
    pub fn is_empty(&self) -> bool {
        self.checks.is_empty()
    }

    /// Run every check in the workspace, returning one result each.
    pub async fn run_all(&self) -> Vec<CheckRunResult> {
        let mut out = Vec::with_capacity(self.checks.len());
        for c in &self.checks {
            out.push(self.run_one(c).await);
        }
        out
    }

    async fn run_one(&self, c: &DeterministicCheck) -> CheckRunResult {
        let started = Instant::now();
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&c.command).current_dir(&self.workspace);
        let fut = cmd.output();
        let (observed, ran_ok) =
            match tokio::time::timeout(Duration::from_millis(c.timeout_ms), fut).await {
                Ok(Ok(o)) => {
                    let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                    s.push_str(&String::from_utf8_lossy(&o.stderr));
                    (s.trim_end().to_string(), true)
                }
                Ok(Err(e)) => (format!("spawn error: {e}"), false),
                Err(_) => ("check timed out".to_string(), false),
            };
        let passed = ran_ok && observed.contains(&c.expected_substring);
        CheckRunResult {
            check: c.name.clone(),
            method: c.method.clone(),
            observed,
            expected: c.expected_substring.clone(),
            covers: c.covers.clone(),
            passed,
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_checks_and_local_overrides_project() {
        let dir = std::env::temp_dir().join(format!("rk-checks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("harness")).unwrap();
        std::fs::create_dir_all(dir.join(".rustykeys")).unwrap();

        std::fs::write(
            dir.join("harness/checks.toml"),
            r#"
[[check]]
name = "unit"
command = "echo project-version"
expected_substring = "project-version"
covers = ["req-1"]

[[check]]
name = "lint"
command = "echo lint-ok"
expected_substring = "lint-ok"
method = "lint"
"#,
        )
        .unwrap();
        // Local overrides "unit" with a passing command; adds nothing else.
        std::fs::write(
            dir.join(".rustykeys/checks.toml"),
            r#"
[[check]]
name = "unit"
command = "echo local-version"
expected_substring = "local-version"
covers = ["req-1", "req-2"]
method = "targeted_test"
"#,
        )
        .unwrap();

        let reg = CheckRegistry::load(&dir).unwrap();
        let results = reg.run_all().await;
        assert_eq!(results.len(), 2); // unit (overridden) + lint

        let unit = results.iter().find(|r| r.check == "unit").unwrap();
        assert!(unit.passed);
        assert_eq!(unit.method, "targeted_test"); // local won
        assert_eq!(unit.covers, vec!["req-1", "req-2"]);

        let lint = results.iter().find(|r| r.check == "lint").unwrap();
        assert!(lint.passed);
        assert_eq!(lint.method, "lint");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn failing_check_is_recorded() {
        let dir = std::env::temp_dir().join(format!("rk-checks-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".rustykeys")).unwrap();
        std::fs::write(
            dir.join(".rustykeys/checks.toml"),
            "[[check]]\nname = \"x\"\ncommand = \"echo nope\"\nexpected_substring = \"yes\"\n",
        )
        .unwrap();
        let reg = CheckRegistry::load(&dir).unwrap();
        let results = reg.run_all().await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_files_are_empty() {
        let dir = std::env::temp_dir().join(format!("rk-checks-none-{}", std::process::id()));
        let reg = CheckRegistry::load(&dir).unwrap();
        assert!(reg.is_empty());
    }
}
