//! **Phase 2's exit criterion**: git worktree isolation, end to end,
//! against a real repository and a real `git`.
//!
//! Worktree isolation is the reason this project exists — it is the one
//! capability nothing on the market combines with Windows support — so
//! these tests are about the property itself, not just about the commands
//! returning zero: a worktree session must genuinely not be able to
//! disturb the repository's own working copy.
//!
//! Every test creates its own throwaway repository. Tests must never run
//! against the repository they live in; see `common::TempRepo`.

mod common;

use std::time::Duration;

use common::*;

#[test]
fn a_worktree_session_gets_its_own_worktree_and_branch() {
    let root = TempRoot::new("wt-create");
    let repo = TempRepo::new("wt-create");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );

    let worktrees = repo.worktrees();
    assert!(
        worktrees.contains(&id),
        "the session's worktree should be registered with the repo:\n{worktrees}"
    );
    assert!(
        repo.branches().contains(&format!("sessionmgr/{id}")),
        "the session should own a namespaced branch:\n{}",
        repo.branches()
    );
    assert!(
        repo.path().join(".sessionmgr-worktrees").join(&id).is_dir(),
        "the worktree directory should exist on disk"
    );

    let listing = session_list(root.path());
    assert!(listing.contains("Worktree"));
    assert!(listing.contains(&format!("sessionmgr/{id}")));
}

#[test]
fn a_worktree_sessions_work_does_not_touch_the_main_working_copy() {
    // The isolation property itself, which is the entire point.
    let root = TempRoot::new("wt-isolate");
    let repo = TempRepo::new("wt-isolate");
    let command = commit_a_file("from-the-session.txt");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &command.iter().map(String::as_str).collect::<Vec<_>>(),
    );

    assert!(
        wait_until(
            || session_status(root.path(), &id) == "finished",
            Duration::from_secs(30),
        ),
        "the session's commit should succeed, but it is {}: {}",
        session_status(root.path(), &id),
        std::fs::read_to_string(
            root.path().join("sessions").join(&id).join("transcript.jsonl")
        )
        .unwrap_or_default()
    );

    // The file exists in the worktree...
    assert!(repo
        .path()
        .join(".sessionmgr-worktrees")
        .join(&id)
        .join("from-the-session.txt")
        .exists());
    // ...and emphatically not in the repository's own working copy.
    assert!(
        !repo.path().join("from-the-session.txt").exists(),
        "an isolated session must not write into the main working copy"
    );
    assert!(
        !repo.log_contains("from-the-session"),
        "the main branch must not have the session's commit until it is merged"
    );
}

#[test]
fn closing_with_merge_fast_forwards_the_work_back() {
    let root = TempRoot::new("wt-merge");
    let repo = TempRepo::new("wt-merge");
    let command = commit_a_file("merged.txt");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &command.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert!(wait_until(
        || session_status(root.path(), &id) == "finished",
        Duration::from_secs(30)
    ));

    assert_success("close --merge", &run(root.path(), &["close", &id, "--merge"]));

    assert!(
        repo.log_contains("add-merged.txt"),
        "the session's commit should now be on the repository's own branch:\n{}",
        repo.branches()
    );
    assert!(
        repo.path().join("merged.txt").exists(),
        "the merged file should be in the main working copy"
    );
    assert_eq!(session_status(root.path(), &id), "merged");
    assert!(
        !repo.worktrees().contains(&id),
        "a merged session's worktree should be gone:\n{}",
        repo.worktrees()
    );
}

#[test]
fn closing_with_discard_throws_the_worktree_and_branch_away() {
    let root = TempRoot::new("wt-discard");
    let repo = TempRepo::new("wt-discard");
    let command = commit_a_file("discarded.txt");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &command.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert!(wait_until(
        || session_status(root.path(), &id) == "finished",
        Duration::from_secs(30)
    ));

    assert_success(
        "close --discard",
        &run(root.path(), &["close", &id, "--discard"]),
    );

    assert_eq!(session_status(root.path(), &id), "discarded");
    assert!(!repo.path().join(".sessionmgr-worktrees").join(&id).exists());
    assert!(
        !repo.branches().contains(&format!("sessionmgr/{id}")),
        "a discarded session's branch should be deleted:\n{}",
        repo.branches()
    );
    assert!(
        !repo.log_contains("add-discarded.txt"),
        "discarded work must not have reached the main branch"
    );
}

#[test]
fn a_bare_close_leaves_the_worktree_alone() {
    // Work is not thrown away on an ambiguous instruction. A bare close
    // stops the processes; the worktree and branch stay until the user
    // says what should happen to them.
    let root = TempRoot::new("wt-bare-close");
    let repo = TempRepo::new("wt-bare-close");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );

    assert_success("close", &run(root.path(), &["close", &id]));

    assert_eq!(session_status(root.path(), &id), "closed");
    assert!(
        repo.path().join(".sessionmgr-worktrees").join(&id).is_dir(),
        "a bare close must not delete the session's work"
    );
    assert!(repo.branches().contains(&format!("sessionmgr/{id}")));
}

#[test]
fn merging_a_diverged_branch_fails_loudly_and_keeps_the_work() {
    // `--ff-only` exists precisely so this fails rather than silently
    // inventing a merge commit while a workspace is being torn down. The
    // work must survive the refusal, or the "safe" option would be the
    // destructive one.
    let root = TempRoot::new("wt-diverged");
    let repo = TempRepo::new("wt-diverged");
    let command = commit_a_file("session-side.txt");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &command.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert!(wait_until(
        || session_status(root.path(), &id) == "finished",
        Duration::from_secs(30)
    ));

    // Move the main branch on independently, so a fast-forward is
    // impossible.
    let out = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "main-side", "--no-gpg-sign"])
        .current_dir(repo.path())
        .env("GIT_AUTHOR_NAME", "sessionmgr tests")
        .env("GIT_AUTHOR_EMAIL", "tests@example.invalid")
        .env("GIT_COMMITTER_NAME", "sessionmgr tests")
        .env("GIT_COMMITTER_EMAIL", "tests@example.invalid")
        .output()
        .expect("run git");
    assert!(out.status.success());

    let result = run(root.path(), &["close", &id, "--merge"]);
    assert!(
        !result.status.success(),
        "a diverged branch must not merge silently"
    );

    // The work is still there, and the session is still closeable.
    assert!(
        repo.branches().contains(&format!("sessionmgr/{id}")),
        "a refused merge must keep the branch:\n{}",
        repo.branches()
    );
    assert!(repo.path().join(".sessionmgr-worktrees").join(&id).is_dir());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("--discard") || stderr.contains("by hand"),
        "the error should say what the user can do next: {stderr}"
    );

    // And --discard still works afterwards, so the session is not stuck.
    assert_success(
        "close --discard",
        &run(root.path(), &["close", &id, "--discard"]),
    );
}

#[test]
fn a_same_directory_session_runs_in_the_repo_and_owns_no_branch() {
    let root = TempRoot::new("wt-samedir");
    let repo = TempRepo::new("wt-samedir");
    let command = commit_a_file("same-dir.txt");
    let id = session_new_in(
        root.path(),
        &["--kind", "same-dir", "--repo", &repo.path_str()],
        &command.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert!(wait_until(
        || session_status(root.path(), &id) == "finished",
        Duration::from_secs(30)
    ));

    // Unisolated by design: it writes straight into the working copy.
    assert!(
        repo.path().join("same-dir.txt").exists(),
        "a same-directory session works in the repository itself"
    );
    assert!(
        !repo.path().join(".sessionmgr-worktrees").exists(),
        "a same-directory session must not create a worktree"
    );
    assert!(session_list(root.path()).contains("SameDirectory"));
}

#[test]
fn closing_a_same_directory_session_never_removes_the_users_repository() {
    // `close --discard` on a same-directory session must not be
    // interpreted as "delete the user's repo". The disposition applies to
    // a worktree this tool created, and there isn't one.
    let root = TempRoot::new("wt-samedir-close");
    let repo = TempRepo::new("wt-samedir-close");
    let id = session_new_in(
        root.path(),
        &["--kind", "same-dir", "--repo", &repo.path_str()],
        &long_running(),
    );

    assert_success(
        "close --discard",
        &run(root.path(), &["close", &id, "--discard"]),
    );

    assert!(
        repo.path().join(".git").exists(),
        "the user's repository must still exist"
    );
    assert_eq!(
        session_status(root.path(), &id),
        "closed",
        "a session owning no branch ends Closed, not Discarded"
    );
}

#[test]
fn a_session_created_from_a_subdirectory_resolves_to_the_repository_root() {
    // A user standing in `src/` should get the same repository as one
    // standing at the top, and the worktree should be placed relative to
    // the root rather than nested inside a subdirectory.
    let root = TempRoot::new("wt-subdir");
    let repo = TempRepo::new("wt-subdir");
    let subdir = repo.path().join("src").join("deep");
    std::fs::create_dir_all(&subdir).expect("mkdir");

    let id = session_new_in(
        root.path(),
        &[
            "--kind",
            "worktree",
            "--repo",
            &subdir.to_string_lossy(),
        ],
        &long_running(),
    );

    assert!(
        repo.path().join(".sessionmgr-worktrees").join(&id).is_dir(),
        "the worktree belongs at the repository root, not under the subdirectory"
    );
    assert!(!subdir.join(".sessionmgr-worktrees").exists());
}

#[test]
fn two_worktree_sessions_on_one_repo_are_independent() {
    // The scaling property: many agents, one repo, no conflicts.
    let root = TempRoot::new("wt-parallel");
    let repo = TempRepo::new("wt-parallel");
    let first = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &commit_a_file("first.txt")
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let second = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &commit_a_file("second.txt")
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    assert_ne!(first, second);

    for id in [&first, &second] {
        assert!(
            wait_until(
                || session_status(root.path(), id) == "finished",
                Duration::from_secs(30)
            ),
            "session {id} is {}",
            session_status(root.path(), id)
        );
    }

    // Each saw only its own file: neither could observe the other's work.
    let dir = repo.path().join(".sessionmgr-worktrees");
    assert!(dir.join(&first).join("first.txt").exists());
    assert!(!dir.join(&first).join("second.txt").exists());
    assert!(dir.join(&second).join("second.txt").exists());
    assert!(!dir.join(&second).join("first.txt").exists());

    // And one can be merged without disturbing the other.
    assert_success(
        "close --merge",
        &run(root.path(), &["close", &first, "--merge"]),
    );
    assert!(repo.log_contains("add-first.txt"));
    assert_eq!(session_status(root.path(), &second), "finished");
}
