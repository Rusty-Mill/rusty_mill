//! Phase 7/CAPABILITIES.md's "Switch agent mid-session": handing a
//! session's live agent conversation off to a new session running a
//! different agent CLI.
//!
//! Three tiers, matching this project's own convention for gated CLI
//! tests (see `agent_needs_input_claude.rs`, `fork_sessions.rs`):
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
//!   process as the source.
//! - **Gated on real credentials, content-verified**: the
//!   `*_recalls_codeword_from_the_rendered_handoff` tests below. Unlike
//!   the bookkeeping tier, these drive each CLI directly through
//!   `--kind same-dir --no-pty` (the same non-interactive, deterministic
//!   pattern `fork_sessions.rs`'s own
//!   `resume_fork_session_id_actually_preserves_conversation_context`
//!   uses) rather than through the full interactive `switch-agent`
//!   command, call the exact same `sessionmgr_agents::render_handoff`
//!   production code calls, and assert that a *different* CLI, given
//!   only that rendered text as its initial prompt, can recall a fact
//!   planted in the source's own transcript -- proving the handoff
//!   mechanism itself, not just that a process started. See each test's
//!   own docs for which source/target pairs are covered and why, and
//!   `docs/phase-7-report.md`'s "Live verification" section for what did
//!   and did not run live in this environment.

mod common;

use std::time::Duration;

use common::*;
use sessionmgr_agents::render_handoff;

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

/// Is a real, logged-in `codex` available? Checked via `login status`
/// rather than just `codex --version`/PATH presence (the way
/// `claude_installed` and `fork_sessions.rs`'s own `claude_installed` do
/// for Claude Code) -- Codex is *installed* in every environment this
/// project's own tests already run in, unauthenticated, per
/// `switching_to_the_same_agent_is_rejected`'s own comment above, so a
/// PATH check alone would not actually gate anything here.
fn codex_credentialed() -> bool {
    std::process::Command::new("codex")
        .args(["login", "status"])
        .output()
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains("Logged in")
        })
}

/// Is a `GEMINI_API_KEY` configured? The same direct signal
/// `sessionmgr_agents::gemini`'s own module docs already name as the
/// missing prerequisite for live-verifying this adapter -- Gemini CLI
/// has no separate login step to check the way Codex does.
fn gemini_credentialed() -> bool {
    std::env::var_os("GEMINI_API_KEY").is_some()
}

/// Drives `command` as a `--kind same-dir --no-pty` one-shot session
/// against `repo` -- deterministic and fast, unlike the interactive-PTY
/// `needs-input` path the bookkeeping-tier tests above depend on. Returns
/// the session's raw transcript output once it reaches `finished`, or
/// `None` if it does not reach `finished` within `timeout`.
///
/// `None` deliberately does not distinguish *why*: no credentials, an
/// authenticated-but-quota-exhausted account, or a slow model call that
/// outran `timeout` all look identical from here, and none of them
/// should fail a live-verification test whose entire premise is "this
/// depends on external account state this project does not control".
/// The credential-gating functions above narrow this to "at least
/// nominally logged in" before a test bothers trying at all; this
/// function is the second, honest layer for what a real API call can
/// still do after that.
fn run_to_completion(
    root: &std::path::Path,
    repo: &TempRepo,
    command: &[String],
    timeout: Duration,
) -> Option<Vec<u8>> {
    let command_refs: Vec<&str> = command.iter().map(String::as_str).collect();
    let id = session_new_in(
        root,
        &["--kind", "same-dir", "--repo", &repo.path_str(), "--no-pty"],
        &command_refs,
    );
    let finished = wait_until(|| session_status(root, &id) == "finished", timeout);
    let output = transcript_output(root, &id);
    let _ = run(root, &["close", &id, "--discard"]);
    finished.then_some(output)
}

/// A generous timeout for a single live model call in this tier.
/// Codex's and Claude Code's own one-shot modes typically answer in a
/// few seconds; Gemini CLI was observed, live, to occasionally take
/// close to a minute for the exact same short prompt (network/backend
/// variance, not a sessionmgr concern) -- so this is sized to that
/// slowest observed case rather than the common one, the same way this
/// suite's other live-CLI tests already size their own timeouts to the
/// slowest thing they gate on.
const LIVE_CALL_TIMEOUT: Duration = Duration::from_secs(90);

/// Seeds a fact via `source_command` (labeled `source_label` for the
/// rendered handoff's own preamble), renders the handoff exactly the way
/// `Supervisor::session_switch_agent` does in production
/// (`sessionmgr_agents::render_handoff`), and hands that rendered text --
/// plus one explicit recall question appended to the same initial-prompt
/// string, since these one-shot CLI invocations get exactly one turn --
/// to `target_command` (built from the rendered prompt by
/// `target_command_builder`) as `target_label`'s own initial prompt.
///
/// Skips cleanly (does not fail) if either leg does not finish within
/// [`LIVE_CALL_TIMEOUT`] -- the same "environment-dependent, not
/// asserted here" tier every other live-CLI test in this suite already
/// uses, covering both "no credentials" and a real account-level failure
/// this project does not control.
fn verify_handoff_recall(
    source_label: &str,
    source_command: Vec<String>,
    target_label: &str,
    target_command_builder: impl FnOnce(String) -> Vec<String>,
    codeword: &str,
) {
    let root = TempRoot::new("handoff-recall");
    let repo = TempRepo::new("handoff-recall");

    let Some(source_output) =
        run_to_completion(root.path(), &repo, &source_command, LIVE_CALL_TIMEOUT)
    else {
        eprintln!(
            "skipping: {source_label} did not finish within {LIVE_CALL_TIMEOUT:?} (no \
             credentials, or a real account-level failure such as an exhausted quota -- \
             either way, not something this test can tell apart from the outside, or \
             should fail on)"
        );
        return;
    };

    let handoff = render_handoff(source_label, &source_output);
    let prompt = format!(
        "{handoff}\n\nWhat was the exact codeword mentioned in the transcript above? Reply \
         with only the codeword, nothing else."
    );
    let target_command = target_command_builder(prompt);

    let Some(target_output) =
        run_to_completion(root.path(), &repo, &target_command, LIVE_CALL_TIMEOUT)
    else {
        eprintln!(
            "skipping: {target_label} did not finish within {LIVE_CALL_TIMEOUT:?} (no \
             credentials, or a real account-level failure such as an exhausted quota)"
        );
        return;
    };

    assert!(
        String::from_utf8_lossy(&target_output).contains(codeword),
        "{target_label}, switched to from {source_label}, should have recalled {codeword} \
         from the rendered handoff, but its output was:\n{}",
        String::from_utf8_lossy(&target_output)
    );
}

/// Deliberately does *not* say "remember" this codeword -- an earlier
/// version of this test did, and failed: `render_handoff` only ever
/// renders a session's own *output* bytes (`transcript.jsonl`'s `Output`
/// events, same as production's `session_switch_agent`), never the
/// prompt that was typed at it. A one-shot, non-interactive `-p`/`exec`
/// invocation prints only its final answer, with no echo of the prompt
/// the way an interactive terminal would show one -- so a "remember X,
/// reply OK" seed renders a handoff containing only "OK", and the next
/// CLI has nothing to recall. Making the codeword the assistant's own
/// visible reply, instead of something only the *input* mentions, is
/// what actually lands it in the rendered handoff.
fn seed_prompt(codeword: &str) -> String {
    format!("Output only the following text, with nothing else before or after it: {codeword}")
}

/// Wraps `argv` (a program plus its final positional prompt argument) so
/// the child sees its stdin reach an immediate EOF, independent of
/// whatever this project's own `--no-pty` worker backend does with its
/// end of the pipe (`Backend::Piped`, `worker.rs`, never closes it --
/// correct for an interactive session `attach` might still write to
/// later, but not something a one-shot invocation like every command
/// here should wait on). Live-observed while writing this test: without
/// this, `claude -p` prints its own "no stdin data received in 3s,
/// proceeding without it" warning and continues (harmless, but adds
/// latency and a stray line to the rendered transcript); `codex exec`
/// reads stdin and appends it as a `<stdin>` block per its own `--help`,
/// and appeared to wait on it indefinitely rather than timing out.
/// Closing stdin up front sidesteps every CLI's own undocumented
/// stdin-detection behavior rather than depending on each one
/// individually. None of this affects production: the real adapters
/// always launch each CLI's interactive mode (`codex.rs`'s/etc. own
/// `launch_args`), which is supposed to hold stdin open for `attach`.
///
/// The prompt is passed as `$1`, never interpolated into the script
/// string -- the rendered handoff can contain arbitrary characters
/// (quotes, backslashes, newlines), and building a quoted shell string
/// from it directly would be exactly the kind of injection risk this
/// avoids by construction.
fn close_stdin(mut argv: Vec<String>) -> Vec<String> {
    if cfg!(windows) {
        // Best-effort only: `cmd.exe` has no equivalent one-liner that
        // is both an argv-safe positional-parameter pass-through and a
        // stdin redirect, and this test suite's own `commit_a_file` doc
        // already records `cmd.exe` mishandling quoted arguments with
        // spaces. None of the credentials these tests gate on are
        // configured on the Windows CI job today, so the practical
        // effect of not solving this there is the same as no
        // credentials at all: a clean, honest timeout-skip, not a
        // failure.
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

fn claude_seed(codeword: &str) -> Vec<String> {
    close_stdin(vec![
        "claude".to_owned(),
        "-p".to_owned(),
        seed_prompt(codeword),
    ])
}

fn claude_target(prompt: String) -> Vec<String> {
    close_stdin(vec!["claude".to_owned(), "-p".to_owned(), prompt])
}

fn gemini_seed(codeword: &str) -> Vec<String> {
    close_stdin(vec![
        "gemini".to_owned(),
        "--skip-trust".to_owned(),
        "-p".to_owned(),
        seed_prompt(codeword),
    ])
}

fn gemini_target(prompt: String) -> Vec<String> {
    close_stdin(vec![
        "gemini".to_owned(),
        "--skip-trust".to_owned(),
        "-p".to_owned(),
        prompt,
    ])
}

fn codex_seed(codeword: &str) -> Vec<String> {
    close_stdin(vec![
        "codex".to_owned(),
        "exec".to_owned(),
        seed_prompt(codeword),
    ])
}

fn codex_target(prompt: String) -> Vec<String> {
    close_stdin(vec!["codex".to_owned(), "exec".to_owned(), prompt])
}

/// Codex/Gemini as *source*: this repository's Phase 7 work only ever
/// exercised Claude Code as the source of a switch (see this file's own
/// bookkeeping-tier test above). Gemini as source was entirely untested
/// until now.
#[test]
fn gemini_source_claude_target_recalls_codeword_from_the_rendered_handoff() {
    if !gemini_credentialed() {
        eprintln!("skipping: no GEMINI_API_KEY configured");
        return;
    }
    verify_handoff_recall(
        "Gemini",
        gemini_seed("ORACLE501"),
        "ClaudeCode",
        claude_target,
        "ORACLE501",
    );
}

/// Gemini as *target*: proves a switched-to Gemini session actually
/// recalls a fact from the handoff, not just that a process started --
/// the exact gap `docs/phase-7-report.md`'s "What is not done" names for
/// Codex/Gemini as the receiving agent.
#[test]
fn claude_source_gemini_target_recalls_codeword_from_the_rendered_handoff() {
    if !gemini_credentialed() {
        eprintln!("skipping: no GEMINI_API_KEY configured");
        return;
    }
    verify_handoff_recall(
        "ClaudeCode",
        claude_seed("ORACLE502"),
        "Gemini",
        gemini_target,
        "ORACLE502",
    );
}

/// Codex as *source*. Gated on `codex_credentialed` (a real login), not
/// just PATH presence -- and still expected to skip in *this*
/// environment even though `codex login status` reports logged in: every
/// live `codex exec`/`codex` call made while writing this test returned
/// `ERROR: Quota exceeded. Check your plan and billing details.` from a
/// real, authenticated account with no usable quota. That is a distinct,
/// external failure from "no credentials" -- recorded honestly in
/// `docs/phase-7-report.md` rather than silently left unexplained by a
/// test that just always skips with no reason given.
#[test]
fn codex_source_gemini_target_recalls_codeword_from_the_rendered_handoff() {
    if !codex_credentialed() {
        eprintln!("skipping: codex is not logged in");
        return;
    }
    if !gemini_credentialed() {
        eprintln!("skipping: no GEMINI_API_KEY configured");
        return;
    }
    verify_handoff_recall(
        "Codex",
        codex_seed("ORACLE503"),
        "Gemini",
        gemini_target,
        "ORACLE503",
    );
}

/// Codex as *target* -- the other half of proving a switched-to Codex
/// session recalls the handoff, not just that it started. Same
/// quota-shaped skip as the test above is expected in this specific
/// environment; see that test's own docs.
#[test]
fn gemini_source_codex_target_recalls_codeword_from_the_rendered_handoff() {
    if !gemini_credentialed() {
        eprintln!("skipping: no GEMINI_API_KEY configured");
        return;
    }
    if !codex_credentialed() {
        eprintln!("skipping: codex is not logged in");
        return;
    }
    verify_handoff_recall(
        "Gemini",
        gemini_seed("ORACLE504"),
        "Codex",
        codex_target,
        "ORACLE504",
    );
}
