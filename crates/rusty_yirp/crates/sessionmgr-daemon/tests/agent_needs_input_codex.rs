//! Validates `Codex::needs_input` against a real, running `codex`
//! session. See `agent_needs_input_claude.rs`'s own docs for why this is
//! gated (skipped, not failed) on the CLI being installed.

mod common;

use std::time::Duration;

use common::*;

fn codex_installed() -> bool {
    std::process::Command::new("codex")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_fresh_codex_session_reaches_needs_input_on_its_own() {
    if !codex_installed() {
        eprintln!("skipping: `codex` is not on PATH");
        return;
    }

    // Same reasoning as the Claude Code test: a fresh worktree
    // guarantees `codex`'s own folder-trust gate, its first prompt in a
    // directory it has never run in.
    let root = TempRoot::new("agent-codex");
    let repo = TempRepo::new("agent-codex");
    let id = session_new_in(
        root.path(),
        &[
            "--kind",
            "worktree",
            "--repo",
            &repo.path_str(),
            "--agent",
            "codex",
        ],
        &[],
    );

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
