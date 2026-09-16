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
//! - **Gated on a real, logged-in `codex`**: Codex's own thread-resume
//!   mechanism, pointed at a native thread id located the same way
//!   `Codex::fork_args` does in production (`docs/phase-6-report.md`,
//!   issue #14) -- verified against a real, running `codex` process.
//!   Skipped cleanly when not logged in.

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

fn codex_credentialed() -> bool {
    std::process::Command::new("codex")
        .args(["login", "status"])
        .output()
        .is_ok_and(|o| {
            // `codex login status` writes its message to *stderr*, not
            // stdout, when run without a TTY attached (confirmed live:
            // `codex login status < /dev/null` produces empty stdout and
            // the real "Logged in..."/"Not logged in" text on stderr) --
            // checking stdout alone always reads as "not logged in"
            // regardless of real state, which would have silently
            // stopped this test from ever running for real again even
            // once real credentials/quota were available.
            o.status.success()
                && (String::from_utf8_lossy(&o.stdout).contains("Logged in")
                    || String::from_utf8_lossy(&o.stderr).contains("Logged in"))
        })
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

// `forking_an_unsupported_agent_names_the_gap_clearly` used to live here,
// verifying that forking a Codex session failed with "Codex does not
// support Fork yet". Removed once Codex gained Fork support (this
// report's own 2026-08-17 update): all three agents this project
// supports now answer `true` to `supports_fork()`, so
// `Supervisor::session_fork`'s own "{agent:?} does not support Fork yet"
// branch has no real adapter left to exercise it against today -- it
// stays in the source as defensive code for a future, genuinely
// unsupported adapter, just without a black-box test that would need to
// fake one to run. `forking_a_codex_session_with_no_conversation_yet_names_the_gap_clearly`
// below covers what this test's own Codex case was actually verifying
// day to day (nothing to fork from *yet*), just correctly attributed now
// that Codex's own gap is "no rollout file located", not "unsupported".

#[test]
fn forking_a_gemini_session_with_no_conversation_yet_names_the_gap_clearly() {
    // Gemini CLI's Fork *mechanism* works (`supports_fork() == true`),
    // but a session that never actually talked to the model has no chat
    // file yet for `fork_args`'s own discovery step to find -- a real,
    // session-specific "cannot fork this one right now" outcome. No live
    // model call happens here at all: the interactive trust gate blocks
    // before any message would ever be sent, so this needs no
    // `GEMINI_API_KEY` and costs no quota.
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

#[test]
fn forking_a_codex_session_with_no_conversation_yet_names_the_gap_clearly() {
    // Codex's Fork *mechanism* works (`supports_fork() == true`), but a
    // session that never actually talked to the model has no rollout
    // file yet for `fork_args`'s own discovery step to find -- a real,
    // session-specific "cannot fork this one right now" outcome, not the
    // same thing as "Codex does not support Fork" (this file's own
    // `forking_an_unsupported_agent_names_the_gap_clearly`, which
    // predates this adapter supporting Fork at all). Confirmed live
    // while building this: a bare interactive `codex` launch does not
    // write a rollout file until an actual conversation happens, so this
    // needs no live model call, no `OPENAI_API_KEY`, and costs no quota
    // -- it works identically whether or not codex is credentialed here.
    let root = TempRoot::new("fork-codex-no-rollout");
    let repo = TempRepo::new("fork-codex-no-rollout");
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

    let output = run(root.path(), &["fork", &id]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be forked right now"),
        "the error should explain there is nothing to fork from yet: {stderr}"
    );
    assert!(
        stderr.contains("Codex"),
        "the error should name the agent: {stderr}"
    );

    let _ = run(root.path(), &["close", &id, "--discard"]);
}

/// Wraps `argv` so the child sees its stdin reach an immediate EOF --
/// `codex exec` reads stdin and appends it as a `<stdin>` block even
/// when a prompt is also given, and this project's own `--no-pty` worker
/// backend never closes a session's stdin pipe on its own (correct for
/// an interactive session `attach` might still write to, not what a
/// one-shot invocation needs). Same fix, same reasoning as
/// `switch_agent.rs`'s own `close_stdin` -- duplicated here rather than
/// shared through `common/`, since this project's own convention (see
/// `fresh_uuid` above) is that each gated-CLI test file re-implements
/// the small, test-specific pieces it needs independently.
fn close_stdin(mut argv: Vec<String>) -> Vec<String> {
    if cfg!(windows) {
        return argv;
    }
    let prompt = argv.pop().expect("argv always has a trailing prompt");
    let program_and_flags = argv.join(" ");
    vec![
        "sh".to_owned(),
        "-c".to_owned(),
        format!("exec {program_and_flags} \"$1\" < /dev/null"),
        "sh".to_owned(),
        prompt,
    ]
}

/// Reads `path`'s first line and, if it is a `SessionMeta` record whose
/// own `cwd` matches `target_cwd`, returns its `id` -- deliberately
/// re-implemented here rather than calling into
/// `sessionmgr_agents::codex` (whose own discovery function is private,
/// and thoroughly unit-tested there against fixtures): this test's whole
/// point is independently proving Codex's own conversation-continuation
/// mechanism actually preserves context, the same reason
/// `gemini_chat_file_for` in this same file re-implements Gemini's own
/// discovery rather than calling into its adapter.
fn codex_thread_id_for(
    codex_home: &std::path::Path,
    workspace_cwd: &std::path::Path,
) -> Option<String> {
    let sessions_dir = codex_home.join("sessions");
    let mut day_dirs = Vec::new();
    for year in std::fs::read_dir(&sessions_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        for month in std::fs::read_dir(year.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            for day in std::fs::read_dir(month.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                if day.file_type().is_ok_and(|t| t.is_dir()) {
                    day_dirs.push(day.path());
                }
            }
        }
    }
    day_dirs.sort_by(|a, b| b.cmp(a));
    let target_cwd = workspace_cwd.to_string_lossy().into_owned();

    for day_dir in day_dirs {
        for entry in std::fs::read_dir(&day_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let is_candidate = entry.file_type().is_ok_and(|t| t.is_file())
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"));
            if !is_candidate {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            let mut first_line = String::new();
            if std::io::BufRead::read_line(&mut std::io::BufReader::new(file), &mut first_line)
                .is_err()
            {
                continue;
            }
            let Ok(record) = serde_json::from_str::<serde_json::Value>(&first_line) else {
                continue;
            };
            if record.get("type").and_then(|t| t.as_str()) != Some("session_meta") {
                continue;
            }
            let Some(payload) = record.get("payload") else {
                continue;
            };
            if payload.get("cwd").and_then(|c| c.as_str()) != Some(target_cwd.as_str()) {
                continue;
            }
            if let Some(id) = payload.get("id").and_then(|i| i.as_str()) {
                return Some(id.to_owned());
            }
        }
    }
    None
}

/// Live-verifies the actual mechanism -- Codex's own thread-continuation
/// primitive, pointed at a native thread id located the same way
/// `Codex::fork_args` does in production -- against a real, running
/// `codex` process, mirroring `resume_fork_session_id_actually_preserves_conversation_context`'s
/// own codeword-recall pattern for Claude Code.
///
/// Uses `codex exec resume <id> <prompt>`, not `codex fork <id>`:
/// `codex exec` (the non-interactive mode this test needs for
/// deterministic, PTY-free driving) has no `fork` subcommand, only
/// `resume`. This tests the same underlying claim Fork depends on --
/// does Codex's own conversation-continuation mechanism, given a
/// discovered native thread id, actually preserve context -- without
/// literally exercising the `fork` subcommand's own lineage-tracking
/// (`forked_from_id`/`parent_thread_id`, per ADR-0003); that specific
/// distinction is not observable through conversation content alone
/// either way. Recorded honestly here rather than implied to be
/// identical.
///
/// Skips cleanly (does not fail) if not logged in, or if either live
/// call does not finish in time -- covers a real, external account-level
/// failure (this session's own exhausted billing quota, at the time of
/// writing) this project does not control, the same tier this suite's
/// other live-gated tests already use.
#[test]
fn codex_resume_actually_preserves_conversation_context_via_discovered_thread_id() {
    if !codex_credentialed() {
        eprintln!("skipping: codex is not logged in");
        return;
    }
    let root = TempRoot::new("fork-codex-mechanism");
    let repo = TempRepo::new("fork-codex-mechanism");
    let live_call_timeout = Duration::from_secs(90);

    let seed_command = close_stdin(vec![
        "codex".to_owned(),
        "exec".to_owned(),
        "Output only the following text, with nothing else before or after it: ORACLE901"
            .to_owned(),
    ]);
    let seed_refs: Vec<&str> = seed_command.iter().map(String::as_str).collect();
    let source_id = session_new_in(
        root.path(),
        &["--kind", "same-dir", "--repo", &repo.path_str(), "--no-pty"],
        &seed_refs,
    );
    let seeded = wait_until(
        || session_status(root.path(), &source_id) == "finished",
        live_call_timeout,
    ) && transcript_contains(root.path(), &source_id, "ORACLE901");
    if !seeded {
        eprintln!(
            "skipping: the seed session did not finish with the expected codeword in this \
             environment (a real account-level failure such as an exhausted quota, most \
             likely -- not something this test can tell apart from the outside, or should \
             fail on)"
        );
        let _ = run(root.path(), &["close", &source_id, "--discard"]);
        return;
    }

    let codex_home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }
    .map(std::path::PathBuf::from)
    .map(|h| h.join(".codex"));
    let workspace_cwd = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_owned());
    let thread_id = codex_home
        .as_deref()
        .and_then(|home| codex_thread_id_for(home, &workspace_cwd));
    let Some(thread_id) = thread_id else {
        eprintln!("skipping: could not locate the source session's own native thread id");
        let _ = run(root.path(), &["close", &source_id, "--discard"]);
        return;
    };
    let _ = run(root.path(), &["close", &source_id, "--discard"]);

    let recall_command = close_stdin(vec![
        "codex".to_owned(),
        "exec".to_owned(),
        "resume".to_owned(),
        thread_id,
        "What was the exact codeword in the loaded session above? Reply with only the \
         codeword, nothing else."
            .to_owned(),
    ]);
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
        transcript_contains(root.path(), &forked_id, "ORACLE901"),
        "a codex session resumed via a discovered thread id should have recalled the \
         codeword from the source session's own conversation history"
    );
    let _ = run(root.path(), &["close", &forked_id, "--discard"]);
}
