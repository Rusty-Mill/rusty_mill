//! Automatic commit system for "never lose work" functionality.
//!
//! [`AutoCommitter`] checks for dirty state and creates auto-commits
//! with generated messages. It is not a background thread — callers
//! invoke [`AutoCommitter::check_and_commit`] on their own schedule.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::engine::GitEngine;
use crate::error::GitError;
use crate::types::RepoState;

/// Automatic committer that stages and commits dirty working trees.
///
/// Create one per session. Call [`check_and_commit`] periodically
/// (from a timer, event loop, or CLI command).
pub struct AutoCommitter {
    repo_root: PathBuf,
    last_commit: Option<Instant>,
    debounce_secs: u64,
}

/// Result of an auto-commit check.
#[derive(Debug, Clone)]
pub struct AutoCommitResult {
    /// Commit hash if a commit was created.
    pub commit_hash: Option<String>,
    /// Generated commit message.
    pub message: Option<String>,
    /// Number of files that were committed.
    pub files_changed: usize,
    /// Whether the commit was skipped due to debounce.
    pub debounced: bool,
    /// Whether the commit was skipped because the repository is mid-operation
    /// (merge, rebase, cherry-pick, revert, or bisect) with unresolved state.
    /// Auto-committing here would silently finalize an unresolved conflict
    /// with conflict-marker garbage baked into history.
    pub skipped_mid_operation: bool,
}

impl AutoCommitter {
    /// Create a new auto-committer for the given repository root.
    #[must_use]
    pub fn new(repo_root: &Path, debounce_secs: u64) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            last_commit: None,
            debounce_secs,
        }
    }

    /// Check if the working tree is dirty and commit if so.
    ///
    /// Respects the debounce window — if a commit was made within
    /// `debounce_secs`, this returns early with `debounced: true`.
    ///
    /// # Errors
    ///
    /// Returns [`GitError`] on any git operation failure.
    pub fn check_and_commit(&mut self) -> Result<AutoCommitResult, GitError> {
        // Check debounce.
        if let Some(last) = self.last_commit {
            if last.elapsed().as_secs() < self.debounce_secs {
                return Ok(AutoCommitResult {
                    commit_hash: None,
                    message: None,
                    files_changed: 0,
                    debounced: true,
                    skipped_mid_operation: false,
                });
            }
        }

        let mut engine = GitEngine::open(&self.repo_root)?;
        let state = engine.state()?;

        // Never auto-commit while a merge/rebase/cherry-pick/revert/bisect is
        // in progress — `GitEngine::commit()` folds `MERGE_HEAD` (etc.) as a
        // second parent and finalizes the operation, which would silently
        // bake unresolved conflict-marker text into history.
        if state.repo_state != RepoState::Clean {
            return Ok(AutoCommitResult {
                commit_hash: None,
                message: None,
                files_changed: 0,
                debounced: false,
                skipped_mid_operation: true,
            });
        }

        if !state.is_dirty {
            return Ok(AutoCommitResult {
                commit_hash: None,
                message: None,
                files_changed: 0,
                debounced: false,
                skipped_mid_operation: false,
            });
        }

        // Collect changed file names for the message.
        let statuses = engine.file_statuses()?;
        let file_count = statuses.len();
        let message = generate_message(&statuses);

        // Stage and commit.
        engine.stage_all()?;
        let hash = engine.commit(&message)?;

        self.last_commit = Some(Instant::now());

        Ok(AutoCommitResult {
            commit_hash: Some(hash),
            message: Some(message),
            files_changed: file_count,
            debounced: false,
            skipped_mid_operation: false,
        })
    }

    /// Notify the auto-committer that a file was saved.
    ///
    /// Resets the debounce timer so the next `check_and_commit`
    /// call within the debounce window will be skipped.
    pub fn notify_save(&mut self) {
        // The debounce timer starts from the last commit, not from the save.
        // A save just signals that more changes may follow — the debounce
        // ensures we don't commit during a rapid-save burst.
    }

    /// Reset the debounce timer, allowing immediate commit.
    pub fn reset_debounce(&mut self) {
        self.last_commit = None;
    }
}

/// Generate an auto-commit message from the changed files.
fn generate_message(statuses: &[crate::types::StatusEntry]) -> String {
    let count = statuses.len();
    if count == 0 {
        return "auto: no changes".to_string();
    }

    let file_list: Vec<String> = statuses
        .iter()
        .take(5)
        .map(|s| {
            s.path.file_name().map_or_else(
                || s.path.to_string_lossy().to_string(),
                |n| n.to_string_lossy().to_string(),
            )
        })
        .collect();

    let suffix = if count > 5 {
        format!(" (+{} more)", count - 5)
    } else {
        String::new()
    };

    format!(
        "auto: {}{}\n\n{count} file(s) changed",
        file_list.join(", "),
        suffix
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn init_repo() -> (tempfile::TempDir, AutoCommitter) {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Auto Test").unwrap();
        config.set_str("user.email", "auto@test.com").unwrap();
        drop(config);
        drop(repo);

        let committer = AutoCommitter::new(dir.path(), 5);
        (dir, committer)
    }

    fn manual_commit(dir: &Path, msg: &str) {
        let mut engine = GitEngine::open(dir).unwrap();
        engine.stage_all().unwrap();
        engine.commit(msg).unwrap();
    }

    #[test]
    fn auto_commit_when_dirty() {
        let (dir, mut committer) = init_repo();
        fs::write(dir.path().join("file.txt"), "content").unwrap();

        let result = committer.check_and_commit().unwrap();
        assert!(result.commit_hash.is_some());
        assert_eq!(result.files_changed, 1);
        assert!(!result.debounced);
        assert!(result.message.as_ref().unwrap().starts_with("auto:"));
    }

    #[test]
    fn auto_commit_when_clean() {
        let (dir, mut committer) = init_repo();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        manual_commit(dir.path(), "initial");

        let result = committer.check_and_commit().unwrap();
        assert!(result.commit_hash.is_none());
        assert_eq!(result.files_changed, 0);
        assert!(!result.debounced);
    }

    #[test]
    fn auto_commit_skips_during_unresolved_merge_conflict() {
        let (dir, mut committer) = init_repo();
        fs::write(dir.path().join("file.txt"), "base\n").unwrap();
        manual_commit(dir.path(), "initial");

        // Diverge onto `feature` with a change to the same line.
        let mut engine = GitEngine::open(dir.path()).unwrap();
        let base_branch = engine.state().unwrap().branch.unwrap();
        engine.create_branch("feature").unwrap();
        engine.switch_branch("feature").unwrap();
        fs::write(dir.path().join("file.txt"), "feature change\n").unwrap();
        engine.stage_all().unwrap();
        engine.commit("feature commit").unwrap();

        // Diverge the base branch too, so the merge conflicts instead of
        // fast-forwarding.
        engine.switch_branch(&base_branch).unwrap();
        fs::write(dir.path().join("file.txt"), "base change\n").unwrap();
        engine.stage_all().unwrap();
        engine.commit("base commit").unwrap();

        // Start the merge — leaves `RepoState::Merge` and conflict markers
        // in the working tree for the caller to resolve.
        let merge_result = engine.merge("feature").unwrap();
        assert!(
            !merge_result.conflicts.is_empty(),
            "expected a merge conflict"
        );
        drop(engine);

        let commits_before = GitEngine::open(dir.path()).unwrap().log(10).unwrap().len();
        let conflicted_content = fs::read_to_string(dir.path().join("file.txt")).unwrap();
        assert!(conflicted_content.contains("<<<<<<<"));

        // The auto-committer must refuse to finalize the unresolved merge.
        let result = committer.check_and_commit().unwrap();
        assert!(result.commit_hash.is_none());
        assert!(result.skipped_mid_operation);
        assert!(!result.debounced);

        // Repository must still be mid-merge and the conflicted file must
        // still carry raw conflict markers — nothing was staged/committed.
        let engine = GitEngine::open(dir.path()).unwrap();
        assert_eq!(engine.state().unwrap().repo_state, RepoState::Merge);
        let content_after = fs::read_to_string(dir.path().join("file.txt")).unwrap();
        assert_eq!(content_after, conflicted_content);
        assert!(content_after.contains("<<<<<<<"));
        let commits_after = engine.log(10).unwrap().len();
        assert_eq!(
            commits_before, commits_after,
            "no new commit should be created while the merge is unresolved"
        );
    }

    #[test]
    fn auto_commit_debounce() {
        let (dir, mut committer) = init_repo();
        committer.debounce_secs = 3600; // 1 hour — always debounce

        fs::write(dir.path().join("file.txt"), "content").unwrap();

        // First commit should succeed.
        let r1 = committer.check_and_commit().unwrap();
        assert!(r1.commit_hash.is_some());

        // Second call within debounce window should be skipped.
        fs::write(dir.path().join("file2.txt"), "more").unwrap();
        let r2 = committer.check_and_commit().unwrap();
        assert!(r2.debounced);
        assert!(r2.commit_hash.is_none());

        // Reset debounce allows commit.
        committer.reset_debounce();
        let r3 = committer.check_and_commit().unwrap();
        assert!(r3.commit_hash.is_some());
    }

    #[test]
    fn auto_commit_message_format() {
        let (dir, mut committer) = init_repo();
        fs::write(dir.path().join("alpha.txt"), "a").unwrap();
        fs::write(dir.path().join("beta.txt"), "b").unwrap();

        let result = committer.check_and_commit().unwrap();
        let msg = result.message.unwrap();
        assert!(
            msg.starts_with("auto:"),
            "message should start with 'auto:', got: {msg}"
        );
        assert!(
            msg.contains("2 file(s) changed"),
            "message should contain file count, got: {msg}"
        );
    }

    #[test]
    fn auto_commit_message_truncates_long_list() {
        let (dir, mut committer) = init_repo();
        for i in 0..8 {
            fs::write(
                dir.path().join(format!("file{i}.txt")),
                format!("content {i}"),
            )
            .unwrap();
        }

        let result = committer.check_and_commit().unwrap();
        let msg = result.message.unwrap();
        assert!(
            msg.contains("(+3 more)"),
            "should truncate after 5 files, got: {msg}"
        );
    }

    #[test]
    fn auto_commit_non_repo_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut committer = AutoCommitter::new(dir.path(), 0);
        assert!(committer.check_and_commit().is_err());
    }
}
