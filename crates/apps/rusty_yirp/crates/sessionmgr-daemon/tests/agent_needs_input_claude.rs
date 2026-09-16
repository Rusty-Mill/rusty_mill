//! Validates `ClaudeCode::needs_input` against a real, running `claude`
//! session -- not just the captured-fixture unit tests in
//! `sessionmgr-agents`. Skipped cleanly, not failed, when `claude` isn't
//! on `PATH` (PLAN.md's own convention for gated adapter tests), so CI
//! is not hostage to the CLI being installed everywhere.

mod common;

use std::time::Duration;

use common::*;

/// Is `claude` actually runnable here?
fn claude_installed() -> bool {
    std::process::Command::new("claude")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_fresh_claude_session_reaches_needs_input_on_its_own() {
    if !claude_installed() {
        eprintln!("skipping: `claude` is not on PATH");
        return;
    }

    // A worktree session, not a plain terminal: `claude` refuses a
    // directory it has never seen before without the trust prompt this
    // test relies on, and a fresh worktree guarantees that every run.
    let root = TempRoot::new("agent-claude");
    let repo = TempRepo::new("agent-claude");
    let id = session_new_in(
        root.path(),
        &[
            "--kind",
            "worktree",
            "--repo",
            &repo.path_str(),
            "--agent",
            "claude",
        ],
        &[],
    );

    // No input sent at all. The trust-folder gate is the first thing an
    // interactive `claude` shows in a directory it has never run in --
    // this is exactly the tier-3 pattern match proving itself against
    // real output, not a scripted answer.
    let reached = wait_until(
        || session_status(root.path(), &id) == "needs-input",
        Duration::from_secs(60),
    );
    assert!(
        reached,
        "session {id} never reached needs-input; last status: {}",
        session_status(root.path(), &id)
    );

    let _ = run(root.path(), &["close", &id, "--discard"]);
}
