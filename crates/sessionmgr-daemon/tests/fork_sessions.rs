//! Phase 6/CAPABILITIES.md's "Fork session": cloning a session's own
//! agent-CLI conversation into a brand-new, independent session.
//!
//! Three tiers, matching this project's own convention for gated CLI
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
//! - **Gated on a real `GEMINI_API_KEY`**: Gemini CLI's own fork
//!   mechanism, `--session-file <path>`, pointed at a real chat file
//!   located the same way `GeminiCli::fork_args` does in production
//!   (`docs/phase-6-report.md`, issue #15) -- verified against a real,
//!   running `gemini` process. Skipped cleanly when no key is
//!   configured.

mod common;

use std::time::Duration;

use common::*;

fn claude_installed() -> bool {
    std::process::Command::new("claude")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn gemini_credentialed() -> bool {
    std::env::var_os("GEMINI_API_KEY").is_some()
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

#[test]
fn forking_a_gemini_session_with_no_conversation_yet_names_the_gap_clearly() {
    // Gemini CLI's Fork *mechanism* works (`supports_fork() == true`),
    // but a session that never actually talked to the model has no chat
    // file yet for `fork_args`'s own discovery step to find -- a real,
    // session-specific "cannot fork this one right now" outcome, not the
    // same thing as "Gemini does not support Fork" (`forking_an_unsupported_agent_names_the_gap_clearly`'s
    // own case above), and the error message must say so rather than
    // conflating the two. No live model call happens here at all: the
    // interactive trust gate blocks before any message would ever be
    // sent, so this needs no `GEMINI_API_KEY` and costs no quota.
    let root = TempRoot::new("fork-gemini-no-chat");
    let repo = TempRepo::new("fork-gemini-no-chat");
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
    let _ = wait_until(
        || session_status(root.path(), &id) != "created",
        Duration::from_secs(15),
    );

    let output = run(root.path(), &["fork", &id]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be forked right now"),
        "the error should explain there is nothing to fork from yet: {stderr}"
    );
    assert!(
        stderr.contains("Gemini"),
        "the error should name the agent: {stderr}"
    );

    let _ = run(root.path(), &["close", &id, "--discard"]);
}

/// Reads gemini-cli's own `projects.json` registry and picks the newest
/// `chats/session-*.jsonl` file for `workspace_cwd` -- deliberately
/// re-implemented here rather than calling into `sessionmgr_agents::gemini`
/// (whose own discovery function is private, and thoroughly unit-tested
/// there against fixtures): this test's whole point is independently
/// proving gemini-cli's own `--session-file` mechanism actually preserves
/// context, the same reason `fresh_uuid` below re-implements a UUID
/// generator instead of calling `sessionmgr_proc::native_session_uuid`
/// directly. A trimmed-down version of `GeminiCli::locate_current_chat_file`'s
/// own algorithm: no `"kind":"main"` filtering, since this test's own
/// scratch repository only ever has one conversation in it.
fn gemini_chat_file_for(workspace_cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }?;
    let gemini_dir = std::path::PathBuf::from(home).join(".gemini");
    let registry_text = std::fs::read_to_string(gemini_dir.join("projects.json")).ok()?;
    let registry: serde_json::Value = serde_json::from_str(&registry_text).ok()?;
    let key = workspace_cwd.to_string_lossy().into_owned();
    let project_name = registry.get("projects")?.get(key.as_str())?.as_str()?;

    let chats_dir = gemini_dir.join("tmp").join(project_name).join("chats");
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(&chats_dir).ok()?.flatten() {
        let path = entry.path();
        let is_candidate = entry.file_type().is_ok_and(|t| t.is_file())
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("session-") && n.ends_with(".jsonl"));
        if !is_candidate {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, path));
        }
    }
    newest.map(|(_, path)| path)
}

/// Live-verifies the actual mechanism -- gemini-cli's own
/// `--session-file <path>`, pointed at the source's own real chat file
/// via the same discovery `GeminiCli::fork_args` performs in production
/// -- against a real, running `gemini` process, mirroring
/// `resume_fork_session_id_actually_preserves_conversation_context`'s own
/// codeword-recall pattern for Claude Code. Driven directly via
/// `--kind same-dir --no-pty`, not through the real `fork` command:
/// `fork` never gives a forked session an extra initial prompt (matching
/// Claude Code's own precedent -- `Supervisor::session_fork` always
/// calls `fork_args` with empty `extra`), so proving *recall*
/// specifically needs a follow-up question this test supplies itself,
/// the same reason the Claude Code mechanism test above does not go
/// through `fork` either.
///
/// Skips cleanly (does not fail) if no `GEMINI_API_KEY` is configured,
/// or if either live call does not finish in time -- covers a real,
/// external account-level failure (an exhausted quota) this project does
/// not control, the same tier `switch_agent.rs`'s own live-gated tests
/// already use.
#[test]
fn gemini_session_file_actually_preserves_conversation_context() {
    if !gemini_credentialed() {
        eprintln!("skipping: no GEMINI_API_KEY configured");
        return;
    }
    let root = TempRoot::new("fork-gemini-mechanism");
    let repo = TempRepo::new("fork-gemini-mechanism");
    let live_call_timeout = Duration::from_secs(90);

    let seed_command = [
        "gemini",
        "--skip-trust",
        "-p",
        "Output only the following text, with nothing else before or after it: ORACLE701",
    ];
    let source_id = session_new_in(
        root.path(),
        &["--kind", "same-dir", "--repo", &repo.path_str(), "--no-pty"],
        &seed_command,
    );
    let seeded = wait_until(
        || session_status(root.path(), &source_id) == "finished",
        live_call_timeout,
    ) && transcript_contains(root.path(), &source_id, "ORACLE701");
    if !seeded {
        eprintln!(
            "skipping: the seed session did not finish with the expected codeword in this \
             environment (no usable credentials, or a real account-level failure such as an \
             exhausted quota -- either way, not something this test can tell apart from the \
             outside, or should fail on)"
        );
        let _ = run(root.path(), &["close", &source_id, "--discard"]);
        return;
    }

    let workspace_cwd = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_owned());
    let Some(chat_file) = gemini_chat_file_for(&workspace_cwd) else {
        eprintln!(
            "skipping: could not locate the source session's own gemini chat file via \
             projects.json"
        );
        let _ = run(root.path(), &["close", &source_id, "--discard"]);
        return;
    };
    let _ = run(root.path(), &["close", &source_id, "--discard"]);

    let recall_command = [
        "gemini".to_owned(),
        "--skip-trust".to_owned(),
        "--session-file".to_owned(),
        chat_file.to_string_lossy().into_owned(),
        "-p".to_owned(),
        "What was the exact codeword in the loaded session above? Reply with only the \
         codeword, nothing else."
            .to_owned(),
    ];
    let recall_refs: Vec<&str> = recall_command.iter().map(String::as_str).collect();
    let forked_id = session_new_in(
        root.path(),
        &["--kind", "same-dir", "--repo", &repo.path_str(), "--no-pty"],
        &recall_refs,
    );
    let finished = wait_until(
        || session_status(root.path(), &forked_id) == "finished",
        live_call_timeout,
    );
    if !finished {
        eprintln!(
            "skipping: the recall session did not finish in this environment (same \
             account-level caveat as the seed leg above)"
        );
        let _ = run(root.path(), &["close", &forked_id, "--discard"]);
        return;
    }

    assert!(
        transcript_contains(root.path(), &forked_id, "ORACLE701"),
        "a gemini session loaded via --session-file should have recalled the codeword from \
         the source session's own conversation history"
    );
    let _ = run(root.path(), &["close", &forked_id, "--discard"]);
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
