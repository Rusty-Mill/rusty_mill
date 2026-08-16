//! Where a session's command actually runs, and the naming policy for
//! the worktrees and branches this tool creates.
//!
//! Pure policy: this module decides *what* a worktree should be called
//! and where it should live. Creating one is `sessionmgr-git`'s job.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session::SessionId;

/// The directory this tool keeps its worktrees in, relative to the
/// repository root.
///
/// Inside the repository rather than in a global state directory,
/// deliberately: `git worktree` records an absolute path in the parent
/// repo's metadata either way, so nothing is gained by hiding them
/// elsewhere, and keeping them adjacent means a user who abandons this
/// tool entirely can find and clean up everything it made with `ls`.
///
/// Dot-prefixed so it is inconspicuous, and a name no plausible project
/// would already be using.
pub const WORKTREE_DIR: &str = ".sessionmgr-worktrees";

/// Prefix for branches this tool creates.
///
/// Namespaced so a user can delete everything this tool made with one
/// `git branch -D sessionmgr/*`, and so it can never collide with a
/// branch a human would name.
pub const BRANCH_PREFIX: &str = "sessionmgr";

/// Where a worktree session's worktree lives.
///
/// The id alone is the leaf name -- no timestamp, no label, nothing
/// descriptive. That is a Windows path-length decision, not an aesthetic
/// one: this path is appended to a target repository path this tool does
/// not control and cannot shorten, and `MAX_PATH` is 260 characters
/// unless every consumer opts out. A 12-character leaf leaves as much of
/// that budget as possible for the repo's own nesting and for the deep
/// paths a build tree adds underneath.
pub fn worktree_dir(repo: &Path, id: &SessionId) -> PathBuf {
    repo.join(WORKTREE_DIR).join(id.as_str())
}

/// The branch created for a worktree session.
pub fn branch_name(id: &SessionId) -> String {
    format!("{BRANCH_PREFIX}/{id}")
}

/// The workspace a session runs against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// The repository the session was created against.
    pub repo: PathBuf,
    /// Where the session's command actually runs. Equal to `repo` for a
    /// `SameDirectory` session; the worktree path for a `Worktree` one.
    pub cwd: PathBuf,
    /// The branch created for this session, if it owns one.
    ///
    /// `Some` only for `Worktree` sessions. It is what `close --merge`
    /// merges and what `close --discard` deletes, and its absence is what
    /// makes those options meaningless for the other kinds.
    pub branch: Option<String>,
}

impl Workspace {
    /// A session isolated in its own worktree and branch.
    pub fn worktree(repo: PathBuf, id: &SessionId) -> Self {
        Workspace {
            cwd: worktree_dir(&repo, id),
            branch: Some(branch_name(id)),
            repo,
        }
    }

    /// A session running directly in the repository's working copy.
    ///
    /// **No collision protection, by design.** Several `SameDirectory`
    /// sessions in one repository share a working copy and an index, and
    /// this tool does nothing to stop them from stepping on each other --
    /// no lock, no `.git/index.lock` mitigation, no serialization. That
    /// matches the model this tool's own design notes describe, where a
    /// same-directory session is the explicitly *unisolated* choice and
    /// the isolated one is right there next to it. Pretending to protect
    /// something this does not protect would be worse than saying so.
    pub fn same_directory(repo: PathBuf) -> Self {
        Workspace {
            cwd: repo.clone(),
            branch: None,
            repo,
        }
    }

    /// Does this workspace own a worktree this tool created and is
    /// therefore responsible for removing?
    pub fn owns_worktree(&self) -> bool {
        self.branch.is_some() && self.cwd != self.repo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> SessionId {
        SessionId::new(1_700_000_000_000, 7)
    }

    #[test]
    fn a_worktree_workspace_runs_in_the_worktree_not_the_repo() {
        let ws = Workspace::worktree(PathBuf::from("/repo"), &id());
        assert_ne!(ws.cwd, ws.repo, "the whole point is that it is elsewhere");
        assert!(ws.cwd.starts_with("/repo"));
        assert!(ws.owns_worktree());
    }

    #[test]
    fn a_same_directory_workspace_runs_in_the_repo_and_owns_no_worktree() {
        let ws = Workspace::same_directory(PathBuf::from("/repo"));
        assert_eq!(ws.cwd, ws.repo);
        assert_eq!(ws.branch, None);
        assert!(
            !ws.owns_worktree(),
            "close must never try to remove the user's actual repository"
        );
    }

    #[test]
    fn branches_are_namespaced_and_worktrees_are_id_named() {
        assert!(branch_name(&id()).starts_with("sessionmgr/"));
        assert!(branch_name(&id()).ends_with(id().as_str()));
        assert_eq!(
            worktree_dir(Path::new("/repo"), &id()),
            PathBuf::from("/repo").join(WORKTREE_DIR).join(id().as_str())
        );
    }

    #[test]
    fn two_sessions_never_share_a_worktree_or_a_branch() {
        let a = SessionId::new(1_700_000_000_000, 1);
        let b = SessionId::new(1_700_000_000_000, 2);
        let repo = PathBuf::from("/repo");
        assert_ne!(worktree_dir(&repo, &a), worktree_dir(&repo, &b));
        assert_ne!(branch_name(&a), branch_name(&b));
    }

    #[test]
    fn the_worktree_path_leaf_stays_short_enough_for_windows() {
        // Everything this tool adds to a repository path it does not
        // control. `MAX_PATH` is 260; if this ever grows past ~40 the
        // long-path manifest stops being a safety margin and becomes a
        // hard requirement.
        let repo = PathBuf::from("C:\\r");
        let added = worktree_dir(&repo, &id()).as_os_str().len() - repo.as_os_str().len();
        assert!(
            added <= 40,
            "sessionmgr adds {added} characters to the user's repo path; keep this small"
        );
    }
}
