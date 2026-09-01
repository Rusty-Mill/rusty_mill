//! Validates `GeminiCli::needs_input` against a real, running `gemini`
//! session -- not just the source-derived fixture unit tests in
//! `sessionmgr-agents::gemini`. Skipped cleanly, not failed, unless
//! `gemini` is both on `PATH` *and* authenticated (PLAN.md's own
//! convention for gated adapter tests, extended here: unlike
//! `claude`/`codex`, `gemini` refuses to reach an interactive screen at
//! all without an auth method configured, so "installed" alone is not
//! enough of a gate for this one).
//!
//! This machine has `gemini` installed but no `GEMINI_API_KEY` (or any
//! other auth method) configured, so this test skips here today -- see
//! `sessionmgr_agents::gemini`'s own module docs for what that means and
//! does not mean for the adapter's confidence. It exists so the day
//! credentials are available -- in CI or on a developer's own machine --
//! this adapter's source-derived patterns get the same live proof
//! `claude`'s and `codex`'s already have, with no further code changes.

mod common;

use std::time::Duration;

use common::*;

/// Is `gemini` both on `PATH` and able to reach an interactive screen?
///
/// A bare `--version` check is not enough here (unlike `claude`/`codex`):
/// `gemini` refuses to run at all without an auth method, so this probes
/// the actual failure mode -- a fast, non-interactive `-p` prompt against
/// a scratch directory succeeds only when real credentials are
/// configured.
fn gemini_authenticated() -> bool {
    std::process::Command::new("gemini")
        .args(["--skip-trust", "-p", "say ok"])
        .current_dir(std::env::temp_dir())
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_fresh_gemini_session_reaches_needs_input_on_its_own() {
    if !gemini_authenticated() {
        eprintln!("skipping: `gemini` is not on PATH, or has no auth method configured");
        return;
    }

    // A worktree session, not a plain terminal: `gemini` refuses a
    // directory it has never seen before without the trust prompt this
    // test relies on, and a fresh worktree guarantees that every run.
    let root = TempRoot::new("agent-gemini");
    let repo = TempRepo::new("agent-gemini");
    let id = session_new_in(
        root.path(),
        &[
            "--kind",
            "worktree",
            "--repo",
            &repo.path_str(),
            "--agent",
            "gemini",
        ],
        &[],
    );

    // No input sent at all. The workspace-trust gate is the first thing
    // an interactive `gemini` shows in a directory it has never run in --
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
