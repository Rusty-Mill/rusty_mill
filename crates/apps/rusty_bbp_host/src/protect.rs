//! Protected test paths (#597, owner answer G2: refuse, don't restore).
//!
//! A claims task's frozen profile set may name `protected` globs. When an
//! agent stores a candidate, every diff in its manifest is checked against
//! them before the core sees the call: a diff that adds, modifies or deletes
//! a protected path is refused `protected_path`, naming each path, and
//! nothing is stored or debited. A change to protected tests goes in as its
//! own claim in the `tests` domain, whose candidate touches only protected
//! paths; once that claim is accepted, its diffs may be listed by other
//! candidates. The core requires that claim to be supported by a principal
//! who authored neither its candidate nor its diffs.

use crate::profiles::path_matches;
use rusty_bbp::claims::TESTS_DOMAIN;
use rusty_bbp::*;
use std::collections::BTreeSet;
use std::path::Path;

/// The paths one diff touches, as `git apply` reads the patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Touched {
    pub diff: ArtId,
    pub paths: Vec<String>,
}

/// Why the rule passes a candidate's protected changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exempt {
    /// The candidate names a claim in the `tests` domain.
    pub tests_claim: bool,
    /// Diffs of the current candidates of accepted `tests` claims.
    pub accepted: BTreeSet<ArtId>,
}

/// The protected paths a candidate's diffs touch without an exemption,
/// sorted and deduplicated. Empty means the candidate may be stored. Pure.
pub fn refused(protected: &[String], touched: &[Touched], exempt: &Exempt) -> Vec<String> {
    let hits = |p: &String| protected.iter().any(|g| path_matches(g, p));
    let only_tests = touched.iter().all(|t| t.paths.iter().all(hits));
    if protected.is_empty() || (exempt.tests_claim && only_tests) {
        return Vec::new();
    }
    let out: BTreeSet<&String> = touched
        .iter()
        .filter(|t| !exempt.accepted.contains(&t.diff))
        .flat_map(|t| t.paths.iter().filter(|p| hits(p)))
        .collect();
    out.into_iter().cloned().collect()
}

/// What `st` exempts for a candidate naming `claim`.
pub fn exempt(st: &TaskState, claim: Option<MsgId>) -> Exempt {
    let in_tests = |c: &claims::ClaimRecord| c.body.domains.iter().any(|d| d == TESTS_DOMAIN);
    let tests_claim = claim.and_then(|id| st.claims.get(id)).is_some_and(in_tests);
    let accepted = st
        .claims
        .claims
        .values()
        .filter(|c| c.status == ClaimStatus::Accepted && in_tests(c))
        .filter_map(|c| c.current)
        .filter_map(|k| match &st.artifacts.get(&k)?.payload {
            ArtifactPayload::Candidate(m) => Some(m.diffs.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    Exempt {
        tests_claim,
        accepted,
    }
}

/// Every path `patch` adds, modifies or deletes, both sides of a rename or
/// copy included. `git apply --numstat -z`, the parser that will apply it,
/// names each patched path but only the new side of a rename, so the old
/// side is read from the `rename from` / `copy from` header lines, which can
/// never be hunk content (a hunk line starts with ` `, `+`, `-` or `\`).
/// Run in `scratch` with repository discovery stopped there: inside a
/// repository's subdirectory `git apply` silently skips paths outside it.
/// A path that is not UTF-8 or that git quotes fails closed, and so does a
/// non-empty patch in which git finds no path.
pub fn patch_paths(scratch: &Path, patch: &[u8]) -> Result<Vec<String>, String> {
    let raw = numstat(scratch, patch)?;
    let out = String::from_utf8(raw).map_err(|_| "a patched path is not UTF-8".to_owned())?;
    let mut paths = numstat_paths(&out)?;
    if paths.is_empty() && !patch.iter().all(u8::is_ascii_whitespace) {
        return Err("git reads no path in this patch; refusing to guess".into());
    }
    paths.extend(rename_sources(patch)?);
    Ok(paths)
}

/// `git apply --numstat -z` of `patch` in `scratch`, with its parent as the
/// discovery ceiling so git never takes an enclosing repository's
/// subdirectory as its prefix.
fn numstat(scratch: &Path, patch: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::{Command as Proc, Stdio};
    // Discovery may look at `scratch` itself (empty) but never above it.
    let scratch = scratch
        .canonicalize()
        .map_err(|e| format!("scratch {}: {e}", scratch.display()))?;
    let ceiling = scratch.parent().unwrap_or(&scratch).to_owned();
    let mut child = Proc::new("git")
        .arg("-C")
        .arg(scratch)
        .args(["apply", "--numstat", "-z", "-"])
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    if let Some(mut pipe) = child.stdin.take() {
        // A git that stops reading early reports its own reason below.
        let _ = pipe.write_all(patch);
    }
    let out = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git apply --numstat: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// The old side of every rename or copy in `patch`. Pure.
fn rename_sources(patch: &[u8]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for line in patch.split(|b| *b == b'\n') {
        let rest = [b"rename from ".as_slice(), b"copy from ".as_slice()]
            .iter()
            .find_map(|h| line.strip_prefix(*h));
        let Some(rest) = rest else {
            continue;
        };
        let path = std::str::from_utf8(rest)
            .map_err(|_| "a renamed path is not UTF-8".to_owned())?
            .trim_end_matches('\r');
        if path.starts_with('"') {
            return Err(format!("quoted renamed path {path}; refusing to guess"));
        }
        out.push(path.to_owned());
    }
    Ok(out)
}

/// Parse `--numstat -z`: `added\tdeleted\tpath\0`, or for a rename or copy
/// `added\tdeleted\t\0old\0new\0`. Pure.
fn numstat_paths(out: &str) -> Result<Vec<String>, String> {
    let mut fields = out.split('\0');
    let mut paths = Vec::new();
    while let Some(line) = fields.next() {
        if line.is_empty() {
            continue;
        }
        let path = line
            .splitn(3, '\t')
            .nth(2)
            .ok_or_else(|| format!("unexpected numstat line {line:?}"))?;
        if !path.is_empty() {
            paths.push(path.to_owned());
            continue;
        }
        for side in ["old", "new"] {
            let p = fields
                .next()
                .filter(|p| !p.is_empty())
                .ok_or_else(|| format!("numstat rename without its {side} path"))?;
            paths.push(p.to_owned());
        }
    }
    Ok(paths)
}

/// The rule for one candidate, computed once and judged per attempt.
#[derive(Clone, Debug)]
pub struct Guard {
    pub protected: Vec<String>,
    pub touched: Vec<Touched>,
    pub claim: Option<MsgId>,
    /// A refusal that holds in every state: the candidate leaves the task's
    /// baseline, or names a diff not stored when `touched` was read.
    pub refused: Option<Rejection>,
}

impl Guard {
    /// `Some` rejection when the candidate may not be stored in `st`.
    pub fn judge(&self, st: &TaskState) -> Option<Rejection> {
        self.refused
            .clone()
            .or_else(|| verdict(st, &self.protected, &self.touched, self.claim))
    }
}

/// A refusal of `manifest` when one of its diffs is not stored in `st`. Its
/// paths could not be read, and a diff stored at that id after the read
/// (by another session) must not pass a guard built without it.
pub fn unresolved(st: &TaskState, manifest: &Candidate) -> Option<Rejection> {
    let missing: Vec<String> = manifest
        .diffs
        .iter()
        .filter(|id| !st.artifacts.contains_key(id))
        .map(|id| id.to_string())
        .collect();
    (!missing.is_empty()).then(|| {
        Rejection::new(
            Code::RefUnresolved,
            format!("diffs not stored: {}", missing.join(", ")),
        )
    })
}

/// The paths each of `manifest`'s diffs touches. Diffs are read from
/// `diffs`, a lookup by id that returns the stored patch bytes; patch
/// content never changes, so this is computed once per call.
pub fn touched(
    manifest: &Candidate,
    scratch: &Path,
    diffs: impl Fn(ArtId) -> Result<Vec<u8>, String>,
) -> Result<Vec<Touched>, String> {
    let mut out = Vec::new();
    for id in &manifest.diffs {
        let patch = diffs(*id)?;
        let paths = patch_paths(scratch, &patch).map_err(|e| format!("{id}: {e}"))?;
        out.push(Touched { diff: *id, paths });
    }
    Ok(out)
}

/// The rule against the state a dispatch attempt commits on: `Some`
/// rejection when `touched` changes protected paths without an exemption
/// that holds in `st`. Pure, so it is re-judged on every reload.
pub fn verdict(
    st: &TaskState,
    protected: &[String],
    touched: &[Touched],
    claim: Option<MsgId>,
) -> Option<Rejection> {
    let hit = refused(protected, touched, &exempt(st, claim));
    if hit.is_empty() {
        return None;
    }
    Some(Rejection::new(
        Code::ProtectedPath,
        format!(
            "protected test paths: {}; change them through a claim in the `{TESTS_DOMAIN}` domain",
            hit.join(", ")
        ),
    ))
}

/// A candidate on a task with protected paths builds on its baseline: an
/// older revision could carry weaker tests that no diff shows. Pure.
pub fn off_baseline(baseline: Option<&str>, manifest: &Candidate) -> Option<Rejection> {
    let b = baseline?;
    (manifest.base != b).then(|| {
        Rejection::new(
            Code::ProtectedPath,
            format!(
                "candidates on this task build on its baseline {b}; base {} is another revision",
                manifest.base
            ),
        )
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn t(diff: u64, paths: &[&str]) -> Touched {
        Touched {
            diff: ArtId(diff),
            paths: paths.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    fn globs() -> Vec<String> {
        vec!["tests/**".into(), "**/conftest.py".into()]
    }

    /// #597 review: a diff id missing when the guard was built stays refused
    /// after another session stores a diff there, whose paths were never read.
    #[test]
    fn a_diff_missing_at_the_read_is_refused_in_every_later_state() {
        let m = Candidate {
            base: "a".repeat(40),
            diffs: vec![ArtId(5)],
            claim: None,
            suite: Suite::Build,
        };
        let mut st = TaskState::new(TaskId::from("t"));
        let guard = Guard {
            protected: globs(),
            touched: vec![t(5, &[])],
            claim: None,
            refused: unresolved(&st, &m),
        };
        assert!(guard
            .judge(&st)
            .is_some_and(|r| r.code == Code::RefUnresolved));
        st.artifacts.insert(
            ArtId(5),
            ArtifactRecord {
                id: ArtId(5),
                author: PrincipalId::from("b"),
                kind: ArtifactKind::Diff,
                blob: BlobRef {
                    sha: Sha256::of(b"tests/core.rs"),
                    len: 13,
                },
                payload: ArtifactPayload::Diff,
                spec: None,
                candidate: None,
                run: None,
            },
        );
        assert!(unresolved(&st, &m).is_none());
        assert!(guard
            .judge(&st)
            .is_some_and(|r| r.code == Code::RefUnresolved));
    }

    #[test]
    fn protected_changes_are_named_and_others_pass() {
        let none = Exempt::default();
        let touched = [t(1, &["src/a.rs", "tests/core.rs"]), t(2, &["conftest.py"])];
        assert_eq!(
            refused(&globs(), &touched, &none),
            ["conftest.py", "tests/core.rs"]
        );
        assert!(refused(&globs(), &[t(1, &["src/a.rs"])], &none).is_empty());
        assert!(refused(&[], &touched, &none).is_empty());
    }

    #[test]
    fn a_tests_claim_may_touch_only_protected_paths() {
        let tests = Exempt {
            tests_claim: true,
            ..Exempt::default()
        };
        assert!(refused(&globs(), &[t(1, &["tests/core.rs"])], &tests).is_empty());
        // Mixed with source changes it is refused like any candidate.
        assert_eq!(
            refused(&globs(), &[t(1, &["tests/core.rs", "src/a.rs"])], &tests),
            ["tests/core.rs"]
        );
    }

    #[test]
    fn accepted_tests_diffs_are_reusable_and_nothing_else() {
        let ex = Exempt {
            tests_claim: false,
            accepted: [ArtId(1)].into(),
        };
        let touched = [t(1, &["tests/core.rs"]), t(3, &["src/a.rs"])];
        assert!(refused(&globs(), &touched, &ex).is_empty());
        let other = [t(1, &["tests/core.rs"]), t(4, &["tests/core.rs"])];
        assert_eq!(refused(&globs(), &other, &ex), ["tests/core.rs"]);
    }

    #[test]
    fn numstat_reads_plain_and_renamed_paths() {
        let out = "1\t0\ttests/new.rs\0-\t-\tbin.dat\0\
                   0\t0\t\0tests/old.rs\0src/moved.rs\0";
        assert_eq!(
            numstat_paths(out).expect("parse"),
            ["tests/new.rs", "bin.dat", "tests/old.rs", "src/moved.rs"]
        );
        assert!(numstat_paths("0\t0\t\0only-old\0").is_err());
        assert!(numstat_paths("garbage\0").is_err());
    }

    #[test]
    fn git_lists_added_deleted_and_renamed_paths() {
        // Inside this repository on purpose: without the ceiling, git would
        // treat it as a subdirectory and skip every path.
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("protect-scratch-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).expect("scratch");
        let patch = b"diff --git a/tests/conftest.py b/tests/conftest.py\n\
new file mode 100644\n\
--- /dev/null\n\
+++ b/tests/conftest.py\n\
@@ -0,0 +1 @@\n\
+--- not a header\n\
diff --git a/tests/core.rs b/tests/core.rs\n\
deleted file mode 100644\n\
--- a/tests/core.rs\n\
+++ /dev/null\n\
@@ -1 +0,0 @@\n\
-x\n\
diff --git a/tests/a.rs b/src/a.rs\n\
similarity index 100%\n\
rename from tests/a.rs\n\
rename to src/a.rs\n";
        let paths = patch_paths(&scratch, patch).expect("numstat");
        let _ = std::fs::remove_dir_all(&scratch);
        assert_eq!(
            paths,
            [
                "tests/conftest.py",
                "tests/core.rs",
                "src/a.rs",
                "tests/a.rs"
            ]
        );
    }

    #[test]
    fn rename_sources_fail_closed_on_quoted_paths() {
        assert_eq!(
            rename_sources(b"copy from a/b\nrename from c\n+rename from d\n").expect("ok"),
            ["a/b", "c"]
        );
        assert!(rename_sources(b"rename from \"t\\tab\"\n").is_err());
    }
}
