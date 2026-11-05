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
/// side of every section is read from the patch itself ([`old_sides`]).
/// Run in `scratch` with repository discovery stopped there: inside a
/// repository's subdirectory `git apply` silently skips paths outside it.
/// A path that is not UTF-8 or that git quotes fails closed, and so does a
/// non-empty patch in which git finds no path.
pub fn patch_paths(scratch: &Path, patch: &[u8]) -> Result<Vec<String>, String> {
    // git reads a header path only up to a NUL; no text patch carries one
    // (binary hunks are base85), so its spellings could never be compared.
    if patch.contains(&0) {
        return Err("a NUL byte in the patch; refusing to guess its paths".into());
    }
    let raw = numstat(scratch, patch)?;
    let out = String::from_utf8(raw).map_err(|_| "a patched path is not UTF-8".to_owned())?;
    let mut paths = numstat_paths(&out)?;
    if paths.is_empty() && !patch.iter().all(u8::is_ascii_whitespace) {
        return Err("git reads no path in this patch; refusing to guess".into());
    }
    for old in old_sides(patch)? {
        if !paths.contains(&old) {
            paths.push(old);
        }
    }
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

/// The old side of every file section in `patch`. Pure. `--numstat`
/// names only the new side, but git also renames a file whose `diff --git`
/// header or `---` line names another old path, with or without `rename
/// from`. So the old side is read from `rename from` / `copy from` lines
/// and from every `---` line followed by a `+++` line (a hunk's removed
/// line can look like one; that only adds a path, so it fails closed). A
/// `diff --git` section showing neither, whose two header paths differ or
/// cannot be told apart, is refused: its source is unknown.
fn old_sides(patch: &[u8]) -> Result<Vec<String>, String> {
    let lines: Vec<&[u8]> = patch.split(|b| *b == b'\n').collect();
    let mut out = Vec::new();
    let mut open: Option<&[u8]> = None; // a `diff --git` header without an old side yet
    for (i, line) in lines.iter().enumerate() {
        if let Some(rest) = line.strip_prefix(b"diff --git ".as_slice()) {
            unknown_source(open)?;
            open = Some(rest);
            continue;
        }
        let renamed = [b"rename from ".as_slice(), b"copy from ".as_slice()]
            .iter()
            .find_map(|h| line.strip_prefix(*h));
        let old = match renamed {
            Some(rest) => Some(text(rest, "renamed")?.to_owned()),
            None => match line.strip_prefix(b"--- ".as_slice()) {
                Some(rest) if lines.get(i + 1).is_some_and(|n| n.starts_with(b"+++ ")) => {
                    let side = text(rest, "old")?;
                    // `a/<path>[\t<date>]`; `/dev/null` is a new file: known, no path.
                    let side = side.split('\t').next().unwrap_or_default();
                    Some(match side {
                        "/dev/null" => String::new(),
                        s => s.split_once('/').map_or(s, |(_, p)| p).to_owned(),
                    })
                }
                _ => None,
            },
        };
        let Some(old) = old else {
            continue;
        };
        open = None;
        if !old.is_empty() {
            out.push(normal(&old)?);
        }
    }
    unknown_source(open)?;
    Ok(out)
}

/// A `diff --git a/X b/X` header names its old side; any other header left
/// without `---` or `rename from` does not, and is refused.
fn unknown_source(header: Option<&[u8]>) -> Result<(), String> {
    let Some(h) = header else {
        return Ok(());
    };
    let h = text(h, "header")?;
    let mid = h.len() / 2;
    let same = h.len() % 2 == 1
        && h.is_char_boundary(mid)
        && h.as_bytes()[mid] == b' '
        && h.starts_with("a/")
        && h[mid + 1..].starts_with("b/")
        && h[2..mid] == h[mid + 3..];
    match same {
        true => Ok(()),
        false => Err(format!(
            "diff --git {h}: its source path is unknown; refusing to guess"
        )),
    }
}

/// `p` spelt as the index path git changes: repeated `/` squashed, as git
/// does. A `.` or `..` segment or a control character (git stops at a NUL)
/// is refused rather than interpreted, so a glob never meets a spelling git
/// would read as another path.
fn normal(p: &str) -> Result<String, String> {
    if p.chars().any(char::is_control) {
        return Err(format!(
            "old path {p:?} holds a control character; refusing to guess"
        ));
    }
    let segs: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() || segs.iter().any(|s| *s == "." || *s == "..") {
        return Err(format!(
            "old path {p:?} is not a plain repository path; refusing to guess"
        ));
    }
    Ok(segs.join("/"))
}

fn text<'a>(raw: &'a [u8], what: &str) -> Result<&'a str, String> {
    let t = std::str::from_utf8(raw)
        .map_err(|_| format!("a {what} path is not UTF-8"))?
        .trim_end_matches('\r');
    if t.starts_with('"') {
        return Err(format!("quoted {what} path {t}; refusing to guess"));
    }
    Ok(t)
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
    /// #598: the tests existing at a brownfield task's base that the diffs
    /// modify or delete.
    pub base_tests: Vec<String>,
}

impl Guard {
    /// `Some` rejection when the candidate may not be stored in `st`.
    pub fn judge(&self, st: &TaskState) -> Option<Rejection> {
        self.refused
            .clone()
            .or_else(|| verdict(st, &self.protected, &self.touched, self.claim))
            .or_else(|| crate::baseline::verdict(st, &self.base_tests, self.claim))
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
            base_tests: vec![],
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

    /// #597 review: git applies `diff --git a/tests/core.txt b/src/core.txt`
    /// as a rename even without `rename from`, and `--numstat` names only
    /// `src/core.txt`; the source is still found.
    #[test]
    fn an_implicit_rename_names_its_protected_source() {
        let scratch = std::env::temp_dir().join(format!("protect-implicit-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).expect("scratch");
        let patch = b"diff --git a/tests/core.txt b/src/core.txt\n\
--- a/tests/core.txt\n\
+++ b/src/core.txt\n\
@@ -1 +1 @@\n\
-old\n\
+new\n";
        let paths = patch_paths(&scratch, patch).expect("numstat");
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(paths.contains(&"tests/core.txt".to_owned()), "{paths:?}");
        assert!(paths.contains(&"src/core.txt".to_owned()), "{paths:?}");
    }

    /// #597 review: git stops a header path at a NUL, so a patch with one
    /// is refused before any path is compared.
    #[test]
    fn a_patch_with_a_nul_byte_is_refused() {
        let scratch = std::env::temp_dir().join(format!("protect-nul-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).expect("scratch");
        let patch = b"diff --git a/tests/core.txt\0x b/src/core.txt\n\
--- a/tests/core.txt\0x\n\
+++ b/src/core.txt\n\
@@ -1 +1 @@\n\
-old\n\
+new\n";
        let err = patch_paths(&scratch, patch).unwrap_err();
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(err.contains("NUL"), "{err}");
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
    fn old_sides_name_every_source_and_fail_closed() {
        assert_eq!(
            old_sides(b"copy from a/b\nrename from c\n+rename from d\n").expect("ok"),
            ["a/b", "c"]
        );
        assert!(old_sides(b"rename from \"t\\tab\"\n").is_err());
        // #597 review: a rename without `rename from` still names its source.
        let implicit = b"diff --git a/tests/core.txt b/src/core.txt\n--- a/tests/core.txt\n+++ b/src/core.txt\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(old_sides(implicit).expect("ok"), ["tests/core.txt"]);
        let dated =
            b"--- a/tests/x.rs\t2026-11-03 01:00:00\n+++ b/tests/x.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(old_sides(dated).expect("ok"), ["tests/x.rs"]);
        let created = b"diff --git a/n.rs b/n.rs\nnew file mode 100644\n--- /dev/null\n+++ b/n.rs\n@@ -0,0 +1 @@\n+a\n";
        assert!(old_sides(created).expect("ok").is_empty());
        // A header alone names its source only when both sides are equal.
        let mode = b"diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n";
        assert!(old_sides(mode).expect("ok").is_empty());
        let moved = b"diff --git a/tests/x b/src/x\nold mode 100644\nnew mode 100755\n";
        assert!(old_sides(moved).unwrap_err().contains("unknown"));
        assert!(old_sides(b"--- \"a/t\\tab\"\n+++ b/x\n").is_err());
        // #597 review: git squashes `//`, so the spelling it changes is read.
        let doubled = b"diff --git a/tests//core.txt b/src/core.txt\n--- a/tests//core.txt\n+++ b/src/core.txt\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(old_sides(doubled).expect("ok"), ["tests/core.txt"]);
        assert_eq!(
            old_sides(b"rename from tests//a.rs\nrename to b.rs\n").expect("ok"),
            ["tests/a.rs"]
        );
        for odd in [
            "tests/./core.txt",
            "tests/../tests/core.txt",
            "tests/core.txt\0ignored",
            "tests/core\u{1}.txt",
        ] {
            let p = format!("--- a/{odd}\n+++ b/src/core.txt\n");
            assert!(old_sides(p.as_bytes()).is_err(), "{odd}");
        }
    }
}
