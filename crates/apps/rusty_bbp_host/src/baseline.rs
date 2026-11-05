//! Brownfield tasks (#598 gap 1): the build suite must be green on the base
//! before a claims task that changes existing code opens.
//!
//! `bbp open --base REF` resolves REF to a full commit, runs the task's
//! frozen profile set on a clean checkout of it in the candidate sandbox, and
//! refuses the open `baseline_red` unless every profile passes. A green run
//! is written to `<dir>/baseline/<sha256(task)>.json`; its SHA-256 is the
//! task's `baseline_digest`, and the commit becomes the profile set's
//! `baseline`, which every candidate must build on.
//!
//! The report also lists the test files that exist at the base. A candidate
//! that modifies or deletes one needs Nano: the host refuses it
//! `base_test_changed` unless its claim is `reversible: false` (owner answer
//! Q2), which the core then never accepts without a human decision.

use crate::profiles::path_matches;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

/// One run of the build suite on the base.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// The full commit id the suite ran on.
    pub base: String,
    /// SHA-256 of the checkout's git tree id; `None` if it never checked out.
    pub tree: Option<Sha256>,
    pub status: RunStatus,
    pub profiles: Vec<ProfileResult>,
    /// Every file at the base that matches the profile set's `test_paths`,
    /// sorted.
    pub tests: Vec<String>,
    /// Why the run could not start, if it could not.
    #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Baseline {
    pub fn green(&self) -> bool {
        self.status == RunStatus::Passed && self.error.is_none()
    }

    /// The `baseline_red` detail: each red profile with its exit code and the
    /// one profile-set edit that would let the task open (owner answer Q1).
    pub fn red_detail(&self) -> String {
        if let Some(e) = &self.error {
            return format!("the build suite could not run on base {}: {e}", self.base);
        }
        let red: Vec<&ProfileResult> = self.profiles.iter().filter(|p| p.failed > 0).collect();
        let names: Vec<String> = red
            .iter()
            .map(|p| format!("{} (exit {})", p.name, p.exit_code))
            .collect();
        let edit = match red.first() {
            Some(p) => format!(
                "fix them on the base, or edit profile `{}` in the profile set to skip its red tests (or remove it) and open again",
                p.name
            ),
            None => "the suite did not pass; check the profile set and open again".into(),
        };
        format!(
            "the build suite is {:?} on base {}: {}. To open: {edit}",
            self.status,
            self.base,
            if names.is_empty() {
                "no profile reported".into()
            } else {
                names.join(", ")
            }
        )
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        rusty_serde::json::to_string(self)
            .map(String::into_bytes)
            .map_err(|e| e.to_string())
    }
}

/// `<dir>/baseline/<sha256(task id)>.json`.
pub fn path(dir: &Path, task: &TaskId) -> PathBuf {
    dir.join("baseline")
        .join(format!("{}.json", Sha256::of(task.0.as_bytes()).hex()))
}

/// Write a green baseline and return its digest. A red one is refused.
pub fn store(dir: &Path, task: &TaskId, b: &Baseline) -> Result<Sha256, String> {
    if !b.green() {
        return Err("only a green baseline is stored".into());
    }
    let bytes = b.bytes()?;
    let p = path(dir, task);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    rusty_atomic_file::write(&p, &bytes).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Sha256::of(&bytes))
}

/// The task's baseline, checked against the digest its log froze.
pub fn load(dir: &Path, task: &TaskId, expected: Sha256) -> Result<Baseline, String> {
    let p = path(dir, task);
    let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    if Sha256::of(&bytes) != expected {
        return Err(format!(
            "{} does not match the task's baseline_digest",
            p.display()
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    rusty_serde::json::from_str(text).map_err(|e| e.to_string())
}

/// REF in `repo` as a full commit id.
pub fn resolve(repo: &Path, reference: &str) -> Result<String, String> {
    let out = Proc::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
        .arg(format!("{reference}^{{commit}}"))
        .output()
        .map_err(|e| format!("git: {e}"))?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !out.status.success() || id.len() != 40 {
        return Err(format!(
            "--base {reference}: not a commit in {}",
            repo.display()
        ));
    }
    Ok(id)
}

/// The files of the checkout at `work` that match `globs`, sorted.
pub fn test_files(work: &Path, globs: &[String]) -> Result<Vec<String>, String> {
    let out = Proc::new("git")
        .arg("-C")
        .arg(work)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut files = BTreeSet::new();
    for raw in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let f = std::str::from_utf8(raw).map_err(|_| "a file at the base is not UTF-8")?;
        if globs.iter().any(|g| path_matches(g, f)) {
            files.insert(f.to_owned());
        }
    }
    Ok(files.into_iter().collect())
}

/// The base tests `touched` modifies or deletes. A diff can add only a path
/// absent at the base (it applies there), so any touched base test is a
/// change to an existing one. Pure.
pub fn changed_base_tests(
    base_tests: &[String],
    touched: &[crate::protect::Touched],
) -> Vec<String> {
    let hits: BTreeSet<&String> = touched
        .iter()
        .flat_map(|t| t.paths.iter())
        .filter(|p| base_tests.binary_search(p).is_ok())
        .collect();
    hits.into_iter().cloned().collect()
}

/// The `base_test_changed` rule for a candidate naming `claim`, judged on
/// `st`: refused when it changes base tests and the claim is reversible.
pub fn verdict(st: &TaskState, changed: &[String], claim: Option<MsgId>) -> Option<Rejection> {
    if changed.is_empty() {
        return None;
    }
    let reversible = claim
        .and_then(|c| st.claims.get(c))
        .is_none_or(|c| c.body.reversible);
    reversible.then(|| {
        Rejection::new(
            Code::BaseTestChanged,
            format!(
                "tests that exist at the base change: {}; that needs Nano, so post the change as a claim with reversible: false",
                changed.join(", ")
            ),
        )
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::protect::Touched;

    fn green() -> Baseline {
        Baseline {
            base: "a".repeat(40),
            tree: Some(Sha256::of(b"tree")),
            status: RunStatus::Passed,
            profiles: vec![ProfileResult {
                name: "test".into(),
                exit_code: 0,
                failed: 0,
            }],
            tests: vec!["tests/a.rs".into(), "tests/b.rs".into()],
            error: None,
        }
    }

    #[test]
    fn a_red_baseline_names_each_red_profile_and_the_edit() {
        let mut b = green();
        b.status = RunStatus::Failed;
        b.profiles.push(ProfileResult {
            name: "lint".into(),
            exit_code: 2,
            failed: 1,
        });
        assert!(!b.green());
        let d = b.red_detail();
        assert!(
            d.contains("lint (exit 2)") && !d.contains("test (exit"),
            "{d}"
        );
        assert!(d.contains("edit profile `lint`"), "{d}");
        b.error = Some("git clone: no such repo".into());
        assert!(b.red_detail().contains("could not run"));
    }

    #[test]
    fn only_a_green_baseline_is_stored_and_it_loads_by_digest() {
        let dir = std::env::temp_dir().join(format!("bbp_baseline_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let task = TaskId("T".into());
        let mut red = green();
        red.status = RunStatus::Failed;
        assert!(store(&dir, &task, &red).is_err());
        let digest = store(&dir, &task, &green()).unwrap();
        assert_eq!(load(&dir, &task, digest).unwrap(), green());
        assert!(load(&dir, &task, Sha256::of(b"other")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_base_test_needs_an_irreversible_claim() {
        let touched = [Touched {
            diff: ArtId(3),
            paths: vec![
                "src/lib.rs".into(),
                "tests/b.rs".into(),
                "tests/new.rs".into(),
            ],
        }];
        let changed = changed_base_tests(&green().tests, &touched);
        assert_eq!(changed, ["tests/b.rs"]);
        let st = TaskState::new(TaskId("T".into()));
        let r = verdict(&st, &changed, None).expect("refused");
        assert_eq!(r.code, Code::BaseTestChanged);
        assert!(r.detail.contains("tests/b.rs"));
        assert!(verdict(&st, &[], None).is_none(), "an added test is fine");
    }
}
