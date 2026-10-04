//! A small git-CLI adapter for candidate harnesses (ADR-0005 §6, §7).
//!
//! Each candidate is edited in a **sparse, detached worktree** that holds
//! only the harness crate. After the proposer has run, [`Worktree::stage`]
//! stages everything it changed, including files it created outside the
//! sparse checkout, and lists the changes; [`violations`] then checks them
//! against the allowlist: only regular files under
//! [`HARNESS_SRC`] may change. Commits are kept under `refs/rsi/...`, never
//! on a branch, so a run never clutters `git branch`; those refs are only
//! ever created, never moved ([`Repo::create_ref`]).
//!
//! Every command runs with hooks disabled, a fixed identity and no commit
//! signing, so the user's git configuration cannot run code or prompt.

use std::path::{Path, PathBuf};
use std::process::Command;

use rsi_core::CommitSha;

use crate::error::RuntimeError;

/// The harness crate, relative to the repository root.
pub const HARNESS_DIR: &str = "crates/apps/rusty_rsi/harness";

/// The only paths a proposer may change.
pub const HARNESS_SRC: &str = "crates/apps/rusty_rsi/harness/src/";

/// A git repository, driven through the `git` command line.
#[derive(Debug, Clone)]
pub struct Repo {
    root: PathBuf,
}

/// One staged change: its path and the file mode it would have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The path, relative to the repository root.
    pub path: String,
    /// The new mode (`100644`, `120000` for a symlink, `000000` when
    /// deleted, ...).
    pub mode: String,
}

/// What a proposer changed in a worktree.
#[derive(Debug, Clone)]
pub struct Staged {
    /// Every changed path.
    pub changes: Vec<Change>,
    /// The changes as a unified diff.
    pub diff: Vec<u8>,
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=rsi",
            "-c",
            "user.email=rsi@localhost",
            "-c",
            "advice.updateSparsePath=false",
        ])
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn run(command: &mut Command, what: &str) -> Result<Vec<u8>, RuntimeError> {
    let output = command
        .output()
        .map_err(|e| RuntimeError::io(format!("running git to {what}"), e))?;
    if !output.status.success() {
        return Err(RuntimeError::Git(format!(
            "{what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

impl Repo {
    /// The repository containing `dir`.
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if `dir` is not inside a git work tree.
    pub fn open(dir: &Path) -> Result<Self, RuntimeError> {
        let root = run(
            git(dir).args(["rev-parse", "--show-toplevel"]),
            "find the repository",
        )?;
        Ok(Self {
            root: PathBuf::from(text(&root)),
        })
    }

    /// The repository's root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The commit `rev` names.
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if it names no commit.
    pub fn resolve(&self, rev: &str) -> Result<CommitSha, RuntimeError> {
        let sha = run(
            git(&self.root).args([
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{rev}^{{commit}}"),
            ]),
            "resolve a commit",
        )?;
        Ok(CommitSha::parse(&text(&sha))?)
    }

    /// A sparse, detached worktree at `commit` in the fresh directory
    /// `path`, holding only [`HARNESS_DIR`].
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if git refuses.
    pub fn worktree(&self, path: &Path, commit: &CommitSha) -> Result<Worktree, RuntimeError> {
        // git resolves a relative path against the repository, not us.
        let path = &std::path::absolute(path)
            .map_err(|e| RuntimeError::io(format!("resolving {}", path.display()), e))?;
        // Forget worktrees whose directories are gone (a crashed run).
        run(
            git(&self.root).args(["worktree", "prune"]),
            "prune stale worktrees",
        )?;
        run(
            git(&self.root)
                .args(["worktree", "add", "--detach", "--no-checkout"])
                .arg(path)
                .arg(commit.as_str()),
            "add a worktree",
        )?;
        let worktree = Worktree {
            repo: self.root.clone(),
            path: path.to_path_buf(),
        };
        run(
            git(path).args([
                "sparse-checkout",
                "set",
                "--no-cone",
                &format!("/{HARNESS_DIR}/"),
            ]),
            "restrict the worktree to the harness",
        )?;
        run(
            git(path).args(["checkout", "--quiet", "--detach", commit.as_str()]),
            "check out the harness",
        )?;
        Ok(worktree)
    }

    /// Creates `name` (a full ref such as `refs/rsi/run/3`) pointing at
    /// `commit`, atomically, failing if it already exists. A ref is never
    /// moved: another run's retained candidates stay reachable.
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if the ref exists or git refuses.
    pub fn create_ref(&self, name: &str, commit: &CommitSha) -> Result<(), RuntimeError> {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = git(&self.root)
            .args(["update-ref", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RuntimeError::io("running git to create a ref", e))?;
        let written = match child.stdin.take() {
            // `create` succeeds only if the ref does not exist yet.
            Some(mut stdin) => writeln!(stdin, "create {name} {}", commit.as_str()),
            None => Err(std::io::Error::other("git stdin was not piped")),
        };
        let output = child
            .wait_with_output()
            .map_err(|e| RuntimeError::io("waiting for git", e))?;
        written.map_err(|e| RuntimeError::io("writing to git", e))?;
        if !output.status.success() {
            return Err(RuntimeError::Git(format!(
                "create {name}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }
}

/// A worktree checked out by [`Repo::worktree`].
#[derive(Debug)]
pub struct Worktree {
    repo: PathBuf,
    path: PathBuf,
}

impl Worktree {
    /// The worktree's root, which the proposer edits.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The harness crate inside the worktree.
    #[must_use]
    pub fn harness(&self) -> PathBuf {
        self.path.join(HARNESS_DIR)
    }

    /// Stages every change, including new files outside the sparse
    /// checkout, and lists them.
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if git refuses.
    pub fn stage(&self) -> Result<Staged, RuntimeError> {
        run(
            git(&self.path).args(["add", "--all", "--sparse"]),
            "stage the proposal",
        )?;
        let raw = run(
            git(&self.path).args(["diff", "--cached", "--raw", "-z", "--no-renames"]),
            "list the proposal's changes",
        )?;
        let diff = run(
            git(&self.path).args(["diff", "--cached", "--no-renames", "--binary"]),
            "diff the proposal",
        )?;
        Ok(Staged {
            changes: parse_raw(&raw)?,
            diff,
        })
    }

    /// Commits the staged changes (even none) and returns the commit.
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if git refuses.
    pub fn commit(&self, message: &str) -> Result<CommitSha, RuntimeError> {
        run(
            git(&self.path).args([
                "commit",
                "--quiet",
                "--allow-empty",
                "--no-verify",
                "-m",
                message,
            ]),
            "commit the proposal",
        )?;
        let sha = run(
            git(&self.path).args(["rev-parse", "HEAD"]),
            "read the commit",
        )?;
        Ok(CommitSha::parse(&text(&sha))?)
    }

    /// Removes the worktree; its commits stay reachable through their refs.
    ///
    /// # Errors
    /// [`RuntimeError::Git`] if git refuses.
    pub fn remove(self) -> Result<(), RuntimeError> {
        run(
            git(&self.repo)
                .args(["worktree", "remove", "--force"])
                .arg(&self.path),
            "remove a worktree",
        )
        .map(|_| ())
    }
}

/// Parses `git diff --raw -z` output: `:<old> <new> <oldsha> <newsha>
/// <status>\0<path>\0` per change.
fn parse_raw(raw: &[u8]) -> Result<Vec<Change>, RuntimeError> {
    let bad = || RuntimeError::Git("unexpected `git diff --raw` output".into());
    let mut fields = raw.split(|&b| b == 0).filter(|f| !f.is_empty());
    let mut changes = Vec::new();
    while let Some(meta) = fields.next() {
        let meta = std::str::from_utf8(meta).map_err(|_| bad())?;
        let mode = meta
            .strip_prefix(':')
            .and_then(|m| m.split(' ').nth(1))
            .ok_or_else(bad)?;
        let path = fields.next().ok_or_else(bad)?;
        changes.push(Change {
            path: String::from_utf8_lossy(path).into_owned(),
            mode: mode.to_owned(),
        });
    }
    Ok(changes)
}

/// The changes the allowlist forbids: anything outside [`HARNESS_SRC`],
/// and any symlink or submodule even inside it. Deleting a source file is
/// allowed.
#[must_use]
pub fn violations(changes: &[Change]) -> Vec<String> {
    changes
        .iter()
        .filter(|change| {
            let inside = change.path.starts_with(HARNESS_SRC);
            let regular = matches!(change.mode.as_str(), "100644" | "100755" | "000000");
            !(inside && regular)
        })
        .map(|change| match change.mode.as_str() {
            "120000" => format!("{} (symlink)", change.path),
            "160000" => format!("{} (submodule)", change.path),
            _ => change.path.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, mode: &str) -> Change {
        Change {
            path: path.to_owned(),
            mode: mode.to_owned(),
        }
    }

    #[test]
    fn parses_raw_diff_output() {
        let raw = b":100644 100644 abc def M\0crates/apps/rusty_rsi/harness/src/lib.rs\0:000000 120000 000 123 A\0a b\0";
        assert_eq!(
            parse_raw(raw).expect("parses"),
            vec![
                change("crates/apps/rusty_rsi/harness/src/lib.rs", "100644"),
                change("a b", "120000"),
            ]
        );
        assert!(parse_raw(b"garbage\0").is_err());
        assert!(parse_raw(b":100644 100644 a b M\0").is_err(), "no path");
        assert_eq!(parse_raw(b"").expect("empty").len(), 0);
    }

    #[test]
    fn only_regular_files_under_the_harness_source_may_change() {
        let src = |name: &str| format!("{HARNESS_SRC}{name}");
        let allowed = [
            change(&src("lib.rs"), "100644"),
            change(&src("agent/new.rs"), "100644"),
            change(&src("old.rs"), "000000"),
            change(&src("run.sh"), "100755"),
        ];
        assert!(violations(&allowed).is_empty());
        let forbidden = [
            change("Cargo.toml", "100644"),
            change("crates/apps/rusty_rsi/harness/Cargo.toml", "100644"),
            change(
                "crates/apps/rusty_rsi/crates/rsi-runtime/tasks/x/private/labels.txt",
                "100644",
            ),
            change("crates/apps/rusty_rsi/harness/srcx/lib.rs", "100644"),
            change(&src("leak.rs"), "120000"),
            change(&src("vendored"), "160000"),
        ];
        let found = violations(&forbidden);
        assert_eq!(found.len(), forbidden.len(), "{found:?}");
        assert!(found[4].ends_with("(symlink)"));
        assert!(found[5].ends_with("(submodule)"));
    }
}
