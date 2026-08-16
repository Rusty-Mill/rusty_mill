//! Git adapter: shells out to the system `git`.
//!
//! # Why shell out rather than use a Rust git library
//!
//! Worktree support is the specific thing this project needs, and it is
//! the least mature corner of the pure-Rust git implementations, while
//! being completely solid in `git` itself. Shelling out is simple,
//! correct, and inherits the user's own git configuration -- credentials,
//! hooks, `core.autocrlf`, `safe.directory` -- rather than reimplementing
//! a subset of it and diverging in ways that only show up on someone
//! else's machine.
//!
//! The cost is a process per operation and text parsing. Both are
//! acceptable at this scale: these run at session creation and teardown,
//! not in a loop.
//!
//! # Why every command is argument-vector-spawned
//!
//! Never through a shell. Repository paths, branch names, and file paths
//! all reach this module from outside, and a shell in the middle would
//! turn any of them into an injection vector. `Command` with explicit
//! args passes them as-is with no interpretation.

use std::path::{Path, PathBuf};
use std::process::Command;

use sessionmgr_core::ports::{ChangedFile, GitError, GitPort};

/// The real implementation, against the `git` on `PATH`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemGit;

/// Runs one git command in `cwd` and returns its stdout.
fn git(operation: &'static str, cwd: &Path, args: &[&str]) -> Result<String, GitError> {
    let mut cmd = Command::new("git");
    // `core.longpaths` is Git for Windows' own opt-in for paths beyond
    // the legacy ~260-character limit -- a *git.exe* setting, separate
    // from both this binary's `longPathAware` manifest (which only
    // covers `sessionmgr.exe`'s own filesystem calls, never a child
    // process's) and the Windows `LongPathsEnabled` registry policy.
    // Measured directly: with the registry policy already on and the
    // manifest embedded, `git init` still failed with "Filename too
    // long" inside a ~250-character worktree path until this was set.
    // Passed per-invocation via `-c` rather than left to the user's
    // global config, so a worktree session under a deeply nested repo
    // does not depend on a setting this tool never asked them to make.
    #[cfg(windows)]
    cmd.args(["-c", "core.longpaths=true"]);
    let output = cmd
        .args(args)
        .current_dir(cwd)
        // Git reads the *invoking* terminal for credential and editor
        // prompts. A daemon has no terminal, so a command that decided to
        // prompt would block a worker forever with nothing visible to
        // answer it. These make git fail fast instead.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| GitError {
            operation,
            // The overwhelmingly likely cause, and the one worth naming.
            message: format!("could not run `git` (is it installed and on PATH?): {e}"),
        })?;
    if !output.status.success() {
        return Err(GitError {
            operation,
            message: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

impl GitPort for SystemGit {
    fn repo_root(&self, path: &Path) -> Result<PathBuf, GitError> {
        // `--show-toplevel` rather than looking for a `.git` directory:
        // it resolves from a subdirectory, and it is correct inside a
        // worktree (where `.git` is a file, not a directory) as well as
        // in a normal checkout.
        let out = git("rev-parse", path, &["rev-parse", "--show-toplevel"])?;
        let root = out.trim();
        if root.is_empty() {
            return Err(GitError {
                operation: "rev-parse",
                message: format!("{} is not inside a git repository", path.display()),
            });
        }
        Ok(PathBuf::from(root))
    }

    fn worktree_add(&self, repo: &Path, worktree: &Path, branch: &str) -> Result<(), GitError> {
        let worktree = worktree.to_string_lossy().into_owned();
        // `-b <branch>` creates the branch as part of adding the
        // worktree, so there is never a window where one exists without
        // the other.
        git(
            "worktree add",
            repo,
            &["worktree", "add", "-b", branch, &worktree],
        )?;
        Ok(())
    }

    fn worktree_remove(&self, repo: &Path, worktree: &Path, force: bool) -> Result<(), GitError> {
        let worktree = worktree.to_string_lossy().into_owned();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&worktree);
        let result = git("worktree remove", repo, &args);
        // `prune` regardless of the outcome: if the directory went away
        // by other means (a user deleting it, or a failed removal that
        // still unlinked it), the parent repo keeps a stale
        // administrative entry that makes every later `worktree add`
        // complain about a path that no longer exists.
        let _ = git("worktree prune", repo, &["worktree", "prune"]);
        result.map(|_| ())
    }

    fn branch_delete(&self, repo: &Path, branch: &str, force: bool) -> Result<(), GitError> {
        let flag = if force { "-D" } else { "-d" };
        git("branch delete", repo, &["branch", flag, branch])?;
        Ok(())
    }

    fn merge_fast_forward_only(&self, repo: &Path, branch: &str) -> Result<(), GitError> {
        // `--ff-only` is the whole point: a diverged branch fails loudly
        // here rather than silently producing a merge commit nobody asked
        // for while a workspace is being torn down.
        git("merge", repo, &["merge", "--ff-only", branch])?;
        Ok(())
    }

    fn changed_files(&self, workspace: &Path) -> Result<Vec<ChangedFile>, GitError> {
        let out = git(
            "status",
            workspace,
            // `--untracked-files=all` so a new directory of files shows
            // as its files rather than as one directory entry -- a
            // session that created ten files should show ten.
            &["status", "--porcelain", "--untracked-files=all"],
        )?;
        Ok(out.lines().filter_map(parse_status_line).collect())
    }

    fn diff(&self, workspace: &Path, path: Option<&str>) -> Result<String, GitError> {
        // `HEAD` rather than a bare `git diff`: a bare diff shows only
        // unstaged changes, so anything the session staged would be
        // invisible in a review view -- the exact opposite of what a
        // reviewer needs.
        let mut args = vec!["diff", "HEAD"];
        if let Some(path) = path {
            // `--` first, so a path that looks like a flag or a branch
            // name is unambiguously a path.
            args.push("--");
            args.push(path);
        }
        git("diff", workspace, &args)
    }
}

/// Parses one `git status --porcelain` line.
///
/// The format is two status characters, a space, then the path. Paths
/// with unusual characters are quoted by git, and renames appear as
/// `old -> new`; both are handled here rather than left to produce
/// nonsense paths.
fn parse_status_line(line: &str) -> Option<ChangedFile> {
    if line.len() < 4 {
        return None;
    }
    let (status, rest) = line.split_at(2);
    let path = rest.trim_start();
    // A rename reports both names; the new one is what a reviewer wants
    // to open.
    let path = match path.split_once(" -> ") {
        Some((_old, new)) => new,
        None => path,
    };
    let path = path.trim_matches('"');
    if path.is_empty() {
        return None;
    }
    Some(ChangedFile {
        status: status.to_owned(),
        path: path.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modified_file_parses() {
        assert_eq!(
            parse_status_line(" M src/lib.rs"),
            Some(ChangedFile {
                status: " M".to_owned(),
                path: "src/lib.rs".to_owned()
            })
        );
    }

    #[test]
    fn an_untracked_file_parses() {
        assert_eq!(
            parse_status_line("?? new.txt").map(|c| c.path),
            Some("new.txt".to_owned())
        );
    }

    #[test]
    fn a_rename_reports_the_new_path() {
        assert_eq!(
            parse_status_line("R  old.rs -> new.rs").map(|c| c.path),
            Some("new.rs".to_owned())
        );
    }

    #[test]
    fn a_quoted_path_is_unquoted() {
        assert_eq!(
            parse_status_line("?? \"file with spaces.txt\"").map(|c| c.path),
            Some("file with spaces.txt".to_owned())
        );
    }

    #[test]
    fn junk_lines_are_skipped_rather_than_producing_empty_paths() {
        for line in ["", "M", "   "] {
            assert_eq!(parse_status_line(line), None, "`{line}` should not parse");
        }
    }

    #[test]
    fn staged_and_unstaged_states_are_preserved_distinctly() {
        // The two columns mean different things; collapsing them would
        // lose information the diff view uses.
        assert_eq!(
            parse_status_line("M  a.rs").map(|c| c.status),
            Some("M ".to_owned())
        );
        assert_eq!(
            parse_status_line(" M a.rs").map(|c| c.status),
            Some(" M".to_owned())
        );
        assert_eq!(
            parse_status_line("MM a.rs").map(|c| c.status),
            Some("MM".to_owned())
        );
    }
}
