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

/// A conservative cutoff, not a precise git-internal constant.
///
/// Git for Windows' own MSYS2 compatibility layer has a fixed-size
/// internal buffer computing a new worktree's `$GIT_DIR`, entirely
/// separate from -- and not fixed by -- `core.longpaths` (set on every
/// invocation above), the OS `LongPathsEnabled` registry policy, or this
/// binary's own `longPathAware` manifest. None of those touch this
/// failure at all. Measured directly
/// (`docs/phase-2-windows-verification.md`): a 186-character worktree
/// path succeeded, a 216-character one failed with `fatal: '$GIT_DIR'
/// too big`. The real ceiling sits somewhere in that unmeasured gap and
/// is git-version-dependent, so this is deliberately a round number with
/// margin on both sides of the measured range, not the exact boundary --
/// see <https://github.com/baileyrd/rusty_yirp/issues/7>.
#[cfg(windows)]
const WORKTREE_PATH_WARN_LEN: usize = 200;

/// Fails fast with an explanation, before `git worktree add` ever runs,
/// once a worktree's path is long enough that Git for Windows is known
/// to fail on it with a cryptic internal error instead. Windows-only:
/// the underlying bug is specific to Git for Windows' own MSYS2 layer,
/// not a general git limitation.
#[cfg(windows)]
fn check_worktree_path_length(worktree: &str) -> Result<(), GitError> {
    let len = worktree.chars().count();
    if len >= WORKTREE_PATH_WARN_LEN {
        return Err(GitError {
            operation: "worktree add",
            message: format!(
                "this worktree's path is {len} characters long ({worktree}), past the point \
                 where Git for Windows' own worktree machinery is known to fail with a cryptic \
                 `$GIT_DIR too big` error rather than this one -- move the repository somewhere \
                 shallower before creating a worktree session against it (see \
                 https://github.com/baileyrd/rusty_yirp/issues/7)"
            ),
        });
    }
    Ok(())
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

    fn worktree_add(
        &self,
        repo: &Path,
        worktree: &Path,
        branch: &str,
        start_point: Option<&str>,
    ) -> Result<(), GitError> {
        let worktree = worktree.to_string_lossy().into_owned();
        #[cfg(windows)]
        check_worktree_path_length(&worktree)?;
        // `-b <branch>` creates the branch as part of adding the
        // worktree, so there is never a window where one exists without
        // the other. `start_point`, appended as the trailing positional
        // `<commit-ish>` only when given, is what lets Fork branch from a
        // source session's own branch tip instead of git's own default
        // (repo's checked-out HEAD) -- see this method's own trait docs.
        let mut args = vec!["worktree", "add", "-b", branch, &worktree];
        if let Some(start_point) = start_point {
            args.push(start_point);
        }
        git("worktree add", repo, &args)?;
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
            //
            // `-z`: NUL-terminated records with paths emitted raw
            // rather than the default quoted-and-C-escaped form. The
            // unadorned `--porcelain` format quotes a path containing
            // an unusual character (a literal `"`, a `\`, non-ASCII
            // bytes under `core.quotepath`) using git's own C-style
            // escaping, which `parse_status_records` would otherwise
            // have to re-implement byte-for-byte to undo correctly --
            // `-z` sidesteps that entirely by never escaping in the
            // first place.
            &["status", "--porcelain", "--untracked-files=all", "-z"],
        )?;
        Ok(parse_status_records(&out))
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

/// Parses the NUL-terminated records produced by `git status --porcelain
/// --untracked-files=all -z`.
///
/// Each record is `XY PATH\0`, except a rename or copy (a status
/// containing `R` or `C`), which is two consecutive records: the new
/// path, then the path it came from. `-z` reverses the usual `old ->
/// new` arrow direction and drops the ` -> ` separator entirely in
/// favor of the second NUL-terminated field (see `git-status(1)`'s own
/// docs on `-z`) -- that second field is not needed here (the new path
/// is what a reviewer wants to open, same as before), so it is consumed
/// and dropped rather than left to be misread as its own record.
///
/// Paths are never quoted or C-escaped in this format, which is the
/// whole reason `changed_files` asks for it over plain `--porcelain`:
/// nothing here has to re-implement git's own quoting rules to undo
/// them.
fn parse_status_records(output: &str) -> Vec<ChangedFile> {
    let mut fields = output.split('\0').filter(|f| !f.is_empty());
    let mut changed = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let (status, rest) = field.split_at(2);
        let path = rest.trim_start();
        if path.is_empty() {
            continue;
        }
        if status.contains('R') || status.contains('C') {
            fields.next();
        }
        changed.push(ChangedFile {
            status: status.to_owned(),
            path: path.to_owned(),
        });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modified_file_parses() {
        assert_eq!(
            parse_status_records(" M src/lib.rs\0"),
            vec![ChangedFile {
                status: " M".to_owned(),
                path: "src/lib.rs".to_owned()
            }]
        );
    }

    #[test]
    fn an_untracked_file_parses() {
        assert_eq!(
            parse_status_records("?? new.txt\0")
                .into_iter()
                .map(|c| c.path)
                .collect::<Vec<_>>(),
            vec!["new.txt".to_owned()]
        );
    }

    #[test]
    fn a_rename_reports_the_new_path() {
        // `-z` reverses the arrow: the *new* path comes first, the old
        // path second, as its own NUL-terminated field rather than
        // joined by " -> ".
        assert_eq!(
            parse_status_records("R  new.rs\0old.rs\0")
                .into_iter()
                .map(|c| c.path)
                .collect::<Vec<_>>(),
            vec!["new.rs".to_owned()]
        );
    }

    #[test]
    fn a_path_containing_a_literal_quote_is_not_mangled() {
        // The exact defect `-z` exists to avoid: plain `--porcelain`
        // would render this file's name as the C-escaped
        // `"say \"hi\".txt"`, and trimming a leading/trailing `"` off
        // that (the old approach) leaves the literal backslashes in the
        // path instead of the real `"` character. `-z` never escapes at
        // all, so the raw quote comes straight through.
        assert_eq!(
            parse_status_records("?? say \"hi\".txt\0")
                .into_iter()
                .map(|c| c.path)
                .collect::<Vec<_>>(),
            vec!["say \"hi\".txt".to_owned()]
        );
    }

    #[test]
    fn junk_records_are_skipped_rather_than_producing_empty_paths() {
        for record in ["", "M", "   "] {
            assert!(
                parse_status_records(record).is_empty(),
                "`{record}` should not parse"
            );
        }
    }

    #[test]
    fn staged_and_unstaged_states_are_preserved_distinctly() {
        // The two columns mean different things; collapsing them would
        // lose information the diff view uses.
        assert_eq!(
            parse_status_records("M  a.rs\0")
                .into_iter()
                .map(|c| c.status)
                .collect::<Vec<_>>(),
            vec!["M ".to_owned()]
        );
        assert_eq!(
            parse_status_records(" M a.rs\0")
                .into_iter()
                .map(|c| c.status)
                .collect::<Vec<_>>(),
            vec![" M".to_owned()]
        );
        assert_eq!(
            parse_status_records("MM a.rs\0")
                .into_iter()
                .map(|c| c.status)
                .collect::<Vec<_>>(),
            vec!["MM".to_owned()]
        );
    }

    #[test]
    fn multiple_records_in_one_stream_all_parse() {
        assert_eq!(
            parse_status_records("M  a.rs\0?? b.txt\0")
                .into_iter()
                .map(|c| c.path)
                .collect::<Vec<_>>(),
            vec!["a.rs".to_owned(), "b.txt".to_owned()]
        );
    }

    #[test]
    #[cfg(windows)]
    fn a_worktree_path_under_the_measured_working_length_is_allowed() {
        // 186 characters measured to actually succeed
        // (docs/phase-2-windows-verification.md); the guard must not
        // reject something known to work.
        let path = "C:\\".to_owned() + &"a".repeat(183);
        assert_eq!(path.len(), 186);
        assert!(check_worktree_path_length(&path).is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn a_worktree_path_at_the_cutoff_is_rejected_with_a_clear_message() {
        let path = "C:\\".to_owned() + &"a".repeat(197);
        assert_eq!(path.len(), WORKTREE_PATH_WARN_LEN);
        let err = check_worktree_path_length(&path).expect_err("must reject at the cutoff");
        assert_eq!(err.operation, "worktree add");
        assert!(err.message.contains("$GIT_DIR too big"));
        assert!(err.message.contains("issues/7"));
    }
}
