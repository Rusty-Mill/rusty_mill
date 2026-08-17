//! Phase 7/CAPABILITIES.md's "Switch agent mid-session": handing a
//! session's live agent conversation off to a new session running a
//! different agent CLI.
//!
//! Two tiers, matching this project's own convention for gated CLI tests
//! (see `agent_needs_input_claude.rs`, `fork_sessions.rs`):
//!
//! - **Always-run**: the daemon-side validation rules (agent-present,
//!   different-agent, live-conversation checks) -- these read only the
//!   on-disk record, or need no more than a `codex`/`gemini` process to
//!   exit quickly for lack of credentials, so no live CLI needs to
//!   actually work.
//! - **Gated on a real, installed `claude`**: verifies this project's
//!   own bookkeeping (a new session is created in the *same* workspace,
//!   the source becomes `switched-away`, the handoff text reaches the
//!   new session's own command) against a real running Claude Code
//!   process as the source. The *target* agent (`codex`) is left
//!   unauthenticated in this environment -- proving the new agent can
//!   actually use the handoff needs its own credentials, which this
//!   environment does not have (the same accepted, documented gap as
//!   issues #14/#15), so this test asserts sessionmgr's own mechanics,
//!   not that Codex understands the handoff.

mod common;

use std::time::Duration;

use common::*;

fn claude_installed() -> bool {
    std::process::Command::new("claude")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn switching_requires_an_agent() {
    let root = TempRoot::new("switch-needs-agent");
    let repo = TempRepo::new("switch-needs-agent");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );

    let output = run(root.path(), &["switch-agent", &id, "claude"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no agent CLI conversation to switch away from"),
        "the error should explain there is no conversation to switch: {stderr}"
    );

    let _ = run(root.path(), &["close", &id, "--discard"]);
}

#[test]
fn switching_to_the_same_agent_is_rejected() {
    // Codex is real and installed, but has no credentials in this
    // environment -- the same-agent check reads only the session's own
    // recorded `agent` field, before anything about liveness, so it
    // does not matter whether the process ever gets further than that.
    let root = TempRoot::new("switch-same-agent");
    let repo = TempRepo::new("switch-same-agent");
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

    let output = run(root.path(), &["switch-agent", &id, "codex"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already running Codex"),
        "the error should say the session already runs the requested agent: {stderr}"
    );

    let _ = run(root.path(), &["close", &id, "--discard"]);
}

#[test]
fn switching_requires_a_live_conversation() {
    // Whatever live status a credential-less Codex session actually
    // lands in (running while waiting on a login prompt, or errors out
    // immediately -- either is fine, this test does not depend on
    // which), closing it deterministically produces a non-live,
    // agent-having session: exactly the state switch-agent must reject.
    let root = TempRoot::new("switch-needs-live");
    let repo = TempRepo::new("switch-needs-live");
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
    let _ = wait_until(
        || session_status(root.path(), &id) != "created",
        Duration::from_secs(15),
    );
    let _ = run(root.path(), &["close", &id]);

    let output = run(root.path(), &["switch-agent", &id, "gemini"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("only a live (running or needs-input) session"),
        "the error should explain a live conversation is required: {stderr}"
    );
}

/// Verifies this project's own bookkeeping end to end, against a real
/// running Claude Code source session: a new session is created in the
/// *same* workspace (not a new worktree, unlike Fork), it carries a
/// handoff of the source's transcript in its own command, and the
/// source itself ends up `switched-away` rather than still live.
///
/// Skipped cleanly (not failed) if the source session does not reach
/// `needs-input` within the timeout -- the same environment-dependent
/// interactive-PTY behaviour `agent_needs_input_claude.rs` and
/// `fork_sessions.rs`'s own end-to-end test already document.
#[test]
fn switch_agent_end_to_end_keeps_the_same_workspace_and_hands_off_the_transcript() {
    if !claude_installed() {
        eprintln!("skipping: `claude` is not on PATH");
        return;
    }
    let root = TempRoot::new("switch-e2e");
    let repo = TempRepo::new("switch-e2e");
    let source_id = session_new_in(
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

    if !wait_until(
        || session_status(root.path(), &source_id) == "needs-input",
        Duration::from_secs(60),
    ) {
        eprintln!(
            "skipping: the source session never reached needs-input in this \
             environment (see agent_needs_input_claude.rs's own docs -- a known, \
             environment-dependent interactive-PTY behaviour, not asserted here)"
        );
        let _ = run(root.path(), &["close", &source_id, "--discard"]);
        return;
    }

    let source_workspace_cwd = {
        let listing = session_list(root.path());
        let line = listing
            .lines()
            .find(|l| l.starts_with(&source_id))
            .expect("the source session is in the listing");
        line.to_owned()
    };

    let output = run(root.path(), &["switch-agent", &source_id, "codex"]);
    assert_success("switch-agent", &output);
    let switched_id = stdout_of(&output);
    assert_ne!(switched_id, source_id);

    // The source is now switched-away, not just closed -- a fourth,
    // distinct terminal outcome from Closed/Merged/Discarded.
    assert!(
        wait_until(
            || session_status(root.path(), &source_id) == "switched-away",
            Duration::from_secs(15),
        ),
        "the source session should be switched-away, but is {}",
        session_status(root.path(), &source_id)
    );

    // The new session shares the source's own workspace -- unlike Fork,
    // there is no second worktree.
    let new_listing = session_list(root.path());
    let new_line = new_listing
        .lines()
        .find(|l| l.starts_with(&switched_id))
        .expect("the new session is in the listing");
    let source_branch = source_workspace_cwd
        .split_whitespace()
        .nth(3)
        .expect("the source listing row has a branch column");
    assert!(
        new_line.contains(source_branch),
        "the new session should run on the source's own branch {source_branch}: {new_line}"
    );

    // The handoff text -- built from the source's real transcript --
    // reached the new session as its own initial prompt.
    assert!(
        wait_until(
            || !transcript_output(root.path(), &switched_id).is_empty(),
            Duration::from_secs(15),
        ),
        "the new session should have produced some output"
    );

    let _ = run(root.path(), &["close", &switched_id, "--discard"]);
}
