//! Phase 5: dependent sessions and dependent terminal sessions
//! (CAPABILITIES.md), end to end against a real repository and a real
//! daemon.
//!
//! Both CAPABILITIES.md capabilities are the same underlying mechanism --
//! `SessionKind::Dependent`, set implicitly by `--parent` -- with only
//! whether `--agent` is given distinguishing "a chained task" from "a
//! terminal alongside a running agent". These tests exercise the shared
//! mechanism: sharing the parent's worktree, waiting (or not) for the
//! parent, and what happens to a waiting session when the parent's
//! worktree disappears.

mod common;

use std::time::Duration;

use common::*;

#[test]
fn a_dependent_session_with_start_now_shares_the_parents_worktree_immediately() {
    let root = TempRoot::new("dep-share");
    let repo = TempRepo::new("dep-share");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );
    assert_eq!(session_status(root.path(), &parent), "running");

    let child_command = commit_a_file("from-the-child.txt");
    let child = session_new_in(
        root.path(),
        &["--parent", &parent, "--start-now"],
        &child_command.iter().map(String::as_str).collect::<Vec<_>>(),
    );

    assert!(
        wait_until(
            || session_status(root.path(), &child) == "finished",
            Duration::from_secs(15),
        ),
        "the dependent session should run immediately with --start-now, but is {}",
        session_status(root.path(), &child)
    );
    // Landed in the *parent's* worktree, not a new one of the child's own.
    assert!(repo
        .path()
        .join(".sessionmgr-worktrees")
        .join(&parent)
        .join("from-the-child.txt")
        .exists());
    assert!(!repo
        .path()
        .join(".sessionmgr-worktrees")
        .join(&child)
        .exists());

    let listing = session_list(root.path());
    assert!(listing.contains("Dependent"));
    assert!(
        listing.contains(&parent),
        "the listing should show the parent id: {listing}"
    );
}

#[test]
fn a_dependent_session_waits_for_its_parent_by_default() {
    let root = TempRoot::new("dep-wait");
    let repo = TempRepo::new("dep-wait");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );

    let child = session_new_in(root.path(), &["--parent", &parent], &long_running());
    // The parent is still running, so the child must not have started.
    assert_eq!(session_status(root.path(), &child), "waiting");

    assert_success("close (parent)", &run(root.path(), &["close", &parent]));
    assert_eq!(
        session_status(root.path(), &parent),
        "closed",
        "a bare close leaves the worktree the child depends on in place"
    );

    assert!(
        wait_until(
            || session_status(root.path(), &child) == "running",
            Duration::from_secs(15),
        ),
        "the child should start once its parent finishes, but is {}",
        session_status(root.path(), &child)
    );
    assert_success("close (child)", &run(root.path(), &["close", &child]));
}

#[test]
fn start_now_promotes_an_already_waiting_session() {
    let root = TempRoot::new("dep-startnow-cmd");
    let repo = TempRepo::new("dep-startnow-cmd");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );
    let child_command = echo("hi");
    let child = session_new_in(
        root.path(),
        &["--parent", &parent],
        &child_command.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(session_status(root.path(), &child), "waiting");

    assert_success("start-now", &run(root.path(), &["start-now", &child]));

    assert!(
        wait_until(
            || session_status(root.path(), &child) == "finished",
            Duration::from_secs(15),
        ),
        "start-now should start the child regardless of the parent, but is {}",
        session_status(root.path(), &child)
    );
    // The parent must not have been touched.
    assert_eq!(session_status(root.path(), &parent), "running");
    assert_success("close (parent)", &run(root.path(), &["close", &parent]));
}

#[test]
fn a_waiting_session_is_closeable_without_ever_starting() {
    let root = TempRoot::new("dep-close-waiting");
    let repo = TempRepo::new("dep-close-waiting");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );
    let child = session_new_in(root.path(), &["--parent", &parent], &long_running());
    assert_eq!(session_status(root.path(), &child), "waiting");

    assert_success(
        "close (waiting child)",
        &run(root.path(), &["close", &child]),
    );
    assert_eq!(session_status(root.path(), &child), "closed");
    // Never having started, it must never have been treated as owning
    // (and discarding) the parent's worktree.
    assert!(repo
        .path()
        .join(".sessionmgr-worktrees")
        .join(&parent)
        .is_dir());

    assert_success(
        "close (parent)",
        &run(root.path(), &["close", &parent, "--discard"]),
    );
}

#[test]
fn a_waiting_session_errors_out_when_the_parents_worktree_is_discarded() {
    // The parent's own close removes the worktree the child was counting
    // on -- there is nowhere left for it to start, so it must fail
    // rather than wait forever or start into a directory that no longer
    // exists.
    let root = TempRoot::new("dep-discarded-parent");
    let repo = TempRepo::new("dep-discarded-parent");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );
    let child = session_new_in(root.path(), &["--parent", &parent], &long_running());
    assert_eq!(session_status(root.path(), &child), "waiting");

    assert_success(
        "close --discard (parent)",
        &run(root.path(), &["close", &parent, "--discard"]),
    );

    assert!(
        wait_until(
            || session_status(root.path(), &child) == "errored",
            Duration::from_secs(15),
        ),
        "the child should fail once its parent's worktree is gone, but is {}",
        session_status(root.path(), &child)
    );
}

#[test]
fn creating_a_dependent_session_against_an_already_gone_parent_is_rejected_up_front() {
    let root = TempRoot::new("dep-eager-reject");
    let repo = TempRepo::new("dep-eager-reject");
    let parent_command = echo("done");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &parent_command
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    assert!(wait_until(
        || session_status(root.path(), &parent) == "finished",
        Duration::from_secs(15),
    ));
    assert_success(
        "close --discard (parent)",
        &run(root.path(), &["close", &parent, "--discard"]),
    );

    let output = run(root.path(), &["new", "--parent", &parent, "--start-now"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no longer exists"),
        "the error should say the parent's worktree is gone: {stderr}"
    );
}

#[test]
fn a_dependent_session_needs_a_worktree_owning_parent() {
    let root = TempRoot::new("dep-bad-parent-kind");
    let repo = TempRepo::new("dep-bad-parent-kind");
    let parent = session_new_in(
        root.path(),
        &["--kind", "same-dir", "--repo", &repo.path_str()],
        &long_running(),
    );

    let output = run(root.path(), &["new", "--parent", &parent, "--start-now"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no worktree to depend on"),
        "the error should explain a same-dir session has nothing to depend on: {stderr}"
    );
}

#[test]
fn a_waiting_sessions_poller_survives_an_unclean_daemon_restart() {
    // A `Waiting` session has no worker -- see `SessionStatus::Waiting`'s
    // own docs -- so what actually needs to survive the daemon dying is
    // the in-memory poller task, which does not. `reconcile_all` must
    // restart an equivalent one on the next daemon startup, or a session
    // created just before an unclean exit would wait forever.
    let root = TempRoot::new("dep-poller-restart");
    let repo = TempRepo::new("dep-poller-restart");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );
    let child = session_new_in(root.path(), &["--parent", &parent], &long_running());
    assert_eq!(session_status(root.path(), &child), "waiting");

    let daemon = daemon_pid(root.path()).expect("a daemon should be recorded after `new`");
    force_kill(daemon);
    assert!(
        wait_until(|| !is_alive(daemon), Duration::from_secs(10)),
        "the daemon should be gone after being killed"
    );

    // A real client command is what actually triggers the transparent
    // daemon auto-start (`session_status`/`daemon_pid` below only read
    // files directly and never start anything) -- `list` is as good a
    // trigger as any.
    let listing = session_list(root.path());
    assert!(
        listing.contains(&child),
        "the child should still be listed: {listing}"
    );

    // The child is still exactly where it was -- waiting, with no
    // worker -- and a genuinely new daemon process answered this time.
    assert_eq!(session_status(root.path(), &child), "waiting");
    let new_daemon = daemon_pid(root.path()).expect("a replacement daemon should be recorded");
    assert_ne!(daemon, new_daemon);

    // Finish the parent; if the replacement daemon restarted the
    // child's poller, it should notice and start it without anything
    // else prompting it.
    assert_success("close (parent)", &run(root.path(), &["close", &parent]));
    assert!(
        wait_until(
            || session_status(root.path(), &child) == "running",
            Duration::from_secs(15),
        ),
        "the replacement daemon should have restarted the child's poller, but it is {}",
        session_status(root.path(), &child)
    );
    assert_success("close (child)", &run(root.path(), &["close", &child]));
}

#[test]
fn kind_and_repo_are_rejected_alongside_parent() {
    let root = TempRoot::new("dep-flag-conflicts");
    let repo = TempRepo::new("dep-flag-conflicts");
    let parent = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );

    let with_kind = run(
        root.path(),
        &["new", "--parent", &parent, "--kind", "worktree"],
    );
    assert!(!with_kind.status.success());

    let with_repo = run(
        root.path(),
        &["new", "--parent", &parent, "--repo", &repo.path_str()],
    );
    assert!(!with_repo.status.success());

    let start_now_without_parent = run(root.path(), &["new", "--start-now"]);
    assert!(!start_now_without_parent.status.success());
}
