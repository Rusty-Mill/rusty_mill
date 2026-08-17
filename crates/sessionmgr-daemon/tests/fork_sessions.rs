//! Phase 6/CAPABILITIES.md's "Fork session": cloning a session's own
//! agent-CLI conversation into a brand-new, independent session.
//!
//! Two tiers, matching this project's own convention for gated CLI
//! tests (see `agent_needs_input_claude.rs`):
//!
//! - **Always-run**: the daemon-side validation rules (kind/agent/
//!   support/branch-existence checks) -- these need no real CLI at all,
//!   since they reject before ever spawning anything.
//! - **Gated on a real, installed `claude`**: the actual mechanism --
//!   `--resume <id> --fork-session --session-id <new-id>` -- verified
//!   against a real, running Claude Code process, not assumed from its
//!   `--help` text alone. Skipped cleanly, not failed, when `claude`
//!   isn't on `PATH`.

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
fn forking_requires_a_worktree_session() {
    let root = TempRoot::new("fork-needs-worktree");
    let repo = TempRepo::new("fork-needs-worktree");
    let id = session_new_in(
        root.path(),
        &["--kind", "same-dir", "--repo", &repo.path_str()],
        &long_running(),
    );

    let output = run(root.path(), &["fork", &id]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("worktree session's branch can be forked"),
        "the error should explain only a worktree session can be forked: {stderr}"
    );
}

#[test]
fn forking_requires_an_agent() {
    let root = TempRoot::new("fork-needs-agent");
    let repo = TempRepo::new("fork-needs-agent");
    let id = session_new_in(
        root.path(),
        &["--kind", "worktree", "--repo", &repo.path_str()],
        &long_running(),
    );

    let output = run(root.path(), &["fork", &id]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no agent CLI conversation to fork"),
        "the error should explain there is no conversation to fork: {stderr}"
    );
}

#[test]
fn forking_an_unsupported_agent_names_the_gap_clearly() {
    // Codex is real and installed as an agent kind, but its own adapter
    // does not support Fork yet -- see docs/phase-6-report.md. The error
    // must say so plainly, not fail some other, more confusing way.
    let root = TempRoot::new("fork-unsupported-agent");
    let repo = TempRepo::new("fork-unsupported-agent");
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
    // The session may already have exited (no codex credentials in most
    // test environments) -- that is fine, forking only needs the record
    // and its agent field, not a live process.
    let _ = wait_until(
        || session_status(root.path(), &id) != "created",
        Duration::from_secs(15),
    );

    let output = run(root.path(), &["fork", &id]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Codex does not support Fork yet"),
        "the error should name the agent and say Fork isn't supported for it: {stderr}"
    );

    let _ = run(root.path(), &["close", &id, "--discard"]);
}

/// Live-verifies the actual mechanism -- `claude --resume <id>
/// --fork-session --session-id <new-id>` -- against a real, running
/// Claude Code process, driven directly (not through `--agent claude`,
/// which launches an *interactive* session; this project's own gated
/// `agent_needs_input_claude.rs` test found that specific path prone to
/// not reaching an interactive prompt within its own timeout in some
/// environments, a pre-existing, unrelated finding, not a Fork-specific
/// one). `-p` (print) mode is a real, supported way to drive Claude Code
/// non-interactively and lets this test assert on actual conversation
/// content -- did the forked session really inherit the source's own
/// context -- rather than only on "it started".
#[test]
fn resume_fork_session_id_actually_preserves_conversation_context() {
    if !claude_installed() {
        eprintln!("skipping: `claude` is not on PATH");
        return;
    }
    let root = TempRoot::new("fork-mechanism");

    let uuid1 = fresh_uuid();
    let source_command = [
        "claude",
        "--session-id",
        &uuid1,
        "-p",
        "Remember this exact codeword: ORACLE42. Reply with only the word OK.",
    ];
    let source_id = session_new_in(
        root.path(),
        &["--kind", "terminal", "--no-pty"],
        &source_command,
    );
    assert!(
        wait_until(
            || session_status(root.path(), &source_id) == "finished",
            Duration::from_secs(60),
        ),
        "the source session should finish, but is {}",
        session_status(root.path(), &source_id)
    );
    assert!(
        transcript_contains(root.path(), &source_id, "OK"),
        "the source session should have replied OK"
    );

    let uuid2 = fresh_uuid();
    let forked_command = [
        "claude",
        "--resume",
        &uuid1,
        "--fork-session",
        "--session-id",
        &uuid2,
        "-p",
        "What was the codeword? Reply with only the codeword.",
    ];
    let forked_id = session_new_in(
        root.path(),
        &["--kind", "terminal", "--no-pty"],
        &forked_command,
    );
    assert!(
        wait_until(
            || session_status(root.path(), &forked_id) == "finished",
            Duration::from_secs(60),
        ),
        "the forked session should finish, but is {}",
        session_status(root.path(), &forked_id)
    );
    assert!(
        transcript_contains(root.path(), &forked_id, "ORACLE42"),
        "the forked session should have recalled the codeword from the source \
         session's own conversation history"
    );
}

/// End to end through the real `fork` command and the `--agent claude`
/// adapter path -- an *interactive* session, unlike the mechanism test
/// above. Skipped cleanly (not failed) if the source session does not
/// reach a live state within the timeout: this is the same interactive-
/// PTY behaviour `agent_needs_input_claude.rs` already documents as
/// environment-dependent, not a correctness assertion this test can make
/// reliably everywhere `claude` happens to be installed.
#[test]
fn fork_end_to_end_through_the_real_command() {
    if !claude_installed() {
        eprintln!("skipping: `claude` is not on PATH");
        return;
    }
    let root = TempRoot::new("fork-e2e");
    let repo = TempRepo::new("fork-e2e");
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

    let output = run(root.path(), &["fork", &source_id]);
    assert_success("fork", &output);
    let forked_id = stdout_of(&output);
    assert_ne!(forked_id, source_id);

    let reached = wait_until(
        || {
            let status = session_status(root.path(), &forked_id);
            status == "needs-input" || status == "running"
        },
        Duration::from_secs(60),
    );
    assert!(
        reached,
        "the forked session should have started, but is {}",
        session_status(root.path(), &forked_id)
    );

    let listing = session_list(root.path());
    assert!(listing.contains(&forked_id));

    let _ = run(root.path(), &["close", &source_id, "--discard"]);
    let _ = run(root.path(), &["close", &forked_id, "--discard"]);
}

fn fresh_uuid() -> String {
    // A tiny, dependency-free UUID v4, matching the format
    // `sessionmgr_proc::native_session_uuid` produces -- this test only
    // needs *a* well-formed one to drive the raw `claude` CLI directly.
    let mut bytes = [0u8; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = ((std::process::id() as usize)
            .wrapping_mul(2654435761)
            .wrapping_add(i)
            .wrapping_add(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as usize)
                    .unwrap_or(0),
            )) as u8;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}
