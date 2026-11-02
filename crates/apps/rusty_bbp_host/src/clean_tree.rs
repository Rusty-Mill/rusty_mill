//! Whether a checkout is exactly its HEAD commit. `build.rs` (for
//! `BBP_BUILD_SHA`) and the `bbp pairs` pin both ask, and both include this
//! file, so the two cannot drift apart.
//!
//! A plain `git status --porcelain` can report clean for a changed tree. Each
//! setting below hid a change on git 2.43 (#1328, #1361), and each is undone:
//!
//! - `status.showUntrackedFiles=no`: status lists no untracked files, and
//!   untracked files are listed separately (`ls-files --others`).
//! - `core.excludesFile` and `.git/info/exclude`: that listing reads only the
//!   repository's `.gitignore` files, not `--exclude-standard`.
//! - `core.fileMode=false`: overridden on Unix, where the mode bit is real.
//! - assume-unchanged and skip-worktree index bits (which `core.ignoreStat`
//!   sets): no flag makes status look past them, so any refuses.
//! - `diff.ignoreSubmodules` and `submodule.<name>.ignore`:
//!   `--ignore-submodules=none`.
//! - `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR` inherited
//!   from a git hook that runs cargo: removed, so git inspects `repo`.

use std::path::Path;
use std::process::Command;

/// Environment that would point git at another repository, tree or index.
const REDIRECTS: [&str; 4] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
];

/// What one git command answered.
pub struct GitOut {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// `git -C repo` with the hiding settings undone. `GIT_OPTIONAL_LOCKS=0`:
/// reading the tree's state must not refresh and rewrite the index it inspects.
pub fn command(repo: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(repo);
    if cfg!(unix) {
        c.args(["-c", "core.fileMode=true"]);
    }
    for var in REDIRECTS {
        c.env_remove(var);
    }
    c.env("GIT_OPTIONAL_LOCKS", "0");
    c
}

/// Run `git args` on `repo` (see [`command`]).
pub fn git(repo: &Path, args: &[&str]) -> Result<GitOut, String> {
    let out = command(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    Ok(GitOut {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    })
}

/// One question about the tree: the lines of `args`' output that `counts`
/// accepts are dirt, reported as `label: <line>`.
struct Check {
    args: &'static [&'static str],
    label: &'static str,
    counts: fn(&str) -> bool,
}

const CHECKS: [Check; 3] = [
    Check {
        args: &[
            "status",
            "--porcelain",
            "--untracked-files=no",
            "--ignore-submodules=none",
        ],
        label: "changed",
        counts: any_line,
    },
    Check {
        args: &[
            "ls-files",
            "--others",
            "--directory",
            "--no-empty-directory",
            "--exclude-per-directory=.gitignore",
        ],
        label: "untracked",
        counts: any_line,
    },
    Check {
        args: &["ls-files", "-v"],
        label: "assume-unchanged or skip-worktree",
        counts: hides_edits,
    },
];

/// Why `repo`'s tree is not exactly HEAD, or `None` when it is. `Err` when git
/// could not answer: an unanswered question is not a clean tree. `repo` may be
/// a subdirectory (`build.rs` passes its crate's); `ls-files` lists only the
/// directory it runs in, so the checks run at the top of the tree.
pub fn dirt(repo: &Path) -> Result<Option<String>, String> {
    let top = git(repo, &["rev-parse", "--show-toplevel"])?;
    if !top.ok {
        return Err(format!("git rev-parse failed: {}", top.stderr));
    }
    let top = Path::new(&top.stdout);
    for check in &CHECKS {
        let out = git(top, check.args)?;
        if !out.ok {
            return Err(format!("git {} failed: {}", check.args[0], out.stderr));
        }
        if let Some(line) = out.stdout.lines().find(|l| (check.counts)(l)) {
            return Ok(Some(format!("{}: {line}", check.label)));
        }
    }
    Ok(None)
}

fn any_line(_: &str) -> bool {
    true
}

/// A `git ls-files -v` line whose tag hides edits: lowercase is
/// assume-unchanged, `S` is skip-worktree.
fn hides_edits(line: &str) -> bool {
    line.chars()
        .next()
        .is_some_and(|tag| tag.is_ascii_lowercase() || tag == 'S')
}
