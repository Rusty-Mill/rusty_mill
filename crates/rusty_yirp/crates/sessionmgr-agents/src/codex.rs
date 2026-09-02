//! The Codex adapter.
//!
//! `needs_input`'s patterns are transcribed from real, measured output,
//! same method as [`crate::claude_code`]. See `docs/phase-3-report.md`
//! for the full capture, including the sandbox interaction discovered
//! while verifying Codex's hooks fire on Windows: hook commands run
//! under the same `--sandbox` policy as the agent's own tool calls, so a
//! restrictive sandbox can make a hook silently fail to do its job. That
//! is a hook-*installation*-time concern (Phase 4), not something this
//! adapter's tier-3 fallback needs to account for.
//!
//! **Fork's own discovery step** (locating the source session's native
//! thread id) is live-confirmed against real rollout files Codex wrote
//! on disk -- see [`fork_args`](Codex::fork_args)'s own docs and issue
//! #14. Full content-preservation through a completed fork remains
//! unverified in this environment specifically because of a real
//! account-level billing-quota block, not a credentials or mechanism gap
//! -- see `docs/phase-6-report.md`'s own dated update.

use sessionmgr_core::ports::{AgentAdapterPort, AgentSignal, ForkSource, HookOutcome};

pub struct Codex;

/// Real prompts this session has actually shown. Unlike Claude Code,
/// Codex's own confirmation dialogs share one consistent imperative
/// phrasing -- `"Press <key> to <verb>"` -- across genuinely different
/// dialogs (captured here from two: the folder-trust gate and the
/// plugin-hooks review screen), which is why the pattern below is a
/// substring of that shape rather than one string per dialog.
const NEEDS_INPUT_MARKERS: &[&str] = &[
    "Do you trust the contents of this directory?",
    "Press enter to continue",
    "Press t to trust all",
    "review hooks",
];

/// Events this adapter installs a hook for.
///
/// `SessionStart` and `Stop` are live-verified (Phase 3's sandbox spike,
/// and again in Phase 4's own hook-install spike). `PermissionRequest`
/// is Codex's own real, named event for exactly the tool-approval
/// dialog `needs_input`'s tier-3 patterns already recognize by text --
/// captured directly in Codex's own hooks-review screen (see
/// `docs/phase-3-report.md`), not independently fired-and-observed
/// here. `SubagentStop` matches PLAN.md's `SubagentFinished` webhook
/// category.
const HOOK_EVENTS: &[&str] = &["SessionStart", "PermissionRequest", "Stop", "SubagentStop"];

impl AgentAdapterPort for Codex {
    fn launch_args(
        &self,
        extra: &[String],
        hooks_enabled: bool,
        _native_id: Option<&str>,
    ) -> Vec<String> {
        // `native_id` is ignored, deliberately: Codex has no flag to let
        // a caller pin a new session's own id at launch (its own
        // `ThreadId` is always self-assigned) -- see `fork_args`'s own
        // docs for what that means for Fork support here.
        let mut args = vec!["codex".to_owned()];
        if hooks_enabled {
            // Both measured in Phase 3's own spike: without
            // `--dangerously-bypass-hook-trust`, Codex blocks the
            // session behind an interactive "review hooks" gate before
            // an installed hook is allowed to run at all -- exactly the
            // opposite of what an unattended notification feature
            // needs. Without `--sandbox danger-full-access`, hook
            // commands (which run under the same sandbox as the
            // agent's own tool calls) silently failed.
            args.push("--dangerously-bypass-hook-trust".to_owned());
            args.push("--sandbox".to_owned());
            args.push("danger-full-access".to_owned());
        }
        args.extend(extra.iter().cloned());
        args
    }

    fn needs_input(&self, screen_text: &str) -> AgentSignal {
        if NEEDS_INPUT_MARKERS.iter().any(|m| screen_text.contains(m)) {
            AgentSignal::NeedsInput
        } else {
            AgentSignal::Running
        }
    }

    fn has_verified_hooks(&self) -> bool {
        // Verified today, live: `SessionStart`/`Stop` hooks configured
        // in `.codex/config.toml` under `--sandbox danger-full-access`
        // both produced real marker files. The default `read-only`
        // sandbox silently swallowed the same hooks -- a real, separate
        // finding recorded in docs/phase-3-report.md, not a reason to
        // call hook support itself unverified.
        true
    }

    fn hook_config(
        &self,
        hook_fire_exe: &std::path::Path,
        session_id: &sessionmgr_core::SessionId,
    ) -> (std::path::PathBuf, String) {
        // TOML *literal* strings (single-quoted): no escaping at all,
        // deliberately, which is what makes a Windows path safe to embed
        // here without hand-rolled quoting logic. The official inline
        // examples use exactly this form for the same reason.
        let mut content = String::from("[features]\nhooks = true\n");
        for event in HOOK_EVENTS {
            content.push_str(&format!(
                "\n[[hooks.{event}]]\n\n[[hooks.{event}.hooks]]\ntype = \"command\"\ncommand = '{} __hook-fire --session-id {session_id} --event {event}'\n",
                hook_fire_exe.display()
            ));
        }
        (
            std::path::PathBuf::from(".codex").join("config.toml"),
            content,
        )
    }

    fn hook_signal(&self, event: &str) -> HookOutcome {
        match event {
            "PermissionRequest" | "Stop" => HookOutcome::Status(AgentSignal::NeedsInput),
            "SubagentStop" => HookOutcome::Notify,
            _ => HookOutcome::Ignore,
        }
    }

    /// `codex fork <id>` is real, live-confirmed via a real installed
    /// `codex fork --help` (issue #14's own 2026-08-17 comment). Reaching
    /// it needs this session's *native* Codex thread id, and Codex has
    /// no launch flag to let a caller pin one the way Claude Code's
    /// `--session-id` does -- confirmed absent from a real `codex
    /// --help`. The id is always self-assigned, so this discovers it
    /// instead, via `locate_native_thread_id`.
    ///
    /// Issue #14 originally described this as needing "a post-spawn
    /// filesystem watch... a genuinely separate piece of machinery" --
    /// that framing assumed the id had to be captured and recorded at
    /// *session-creation* time, mirroring Claude Code's pin-at-launch
    /// design. It doesn't: Codex's own rollout file, with a usable
    /// `SessionMeta.cwd`/`id`, is live-confirmed to already exist by the
    /// time anyone would ask to fork a real conversation (written before
    /// the model call completes or even fails). So, like Gemini CLI's
    /// own `locate_current_chat_file`, this is a lazy, synchronous lookup
    /// at *fork* time, not a watch -- no new async machinery, no
    /// `session_fork` changes beyond what `ForkSource` already provides.
    ///
    /// Ignores `new_native_id` entirely, same reason `launch_args`
    /// ignores `native_id`: Codex has no way to pin the *forked*
    /// session's own new thread id either.
    ///
    /// `None` when no rollout file's `SessionMeta.cwd` matches
    /// `source.workspace_cwd` within `locate_native_thread_id`'s own
    /// scan window -- a session-specific "nothing to fork from yet"
    /// outcome, not a statement that Codex's Fork mechanism doesn't
    /// work; see [`ForkSource`]'s and [`AgentAdapterPort::supports_fork`]'s
    /// own docs for why the two are different questions.
    fn fork_args(
        &self,
        source: ForkSource<'_>,
        _new_native_id: &str,
        extra: &[String],
    ) -> Option<Vec<String>> {
        let codex_home = codex_home_dir()?;
        let thread_id = locate_native_thread_id(&codex_home, source.workspace_cwd)?;
        let mut args = vec!["codex".to_owned(), "fork".to_owned(), thread_id];
        args.extend(extra.iter().cloned());
        Some(args)
    }

    fn supports_fork(&self) -> bool {
        // The mechanism works (see `fork_args`'s own docs) -- whether any
        // single call succeeds depends on whether a matching rollout file
        // has been located yet for that call's own workspace, which is
        // not this method's question. See
        // `AgentAdapterPort::supports_fork`'s own docs for why these are
        // deliberately different questions for a discovery-based adapter.
        true
    }
}

/// `codex`'s own global config directory. `CODEX_HOME`, if set (real:
/// `codex --help` itself documents `$CODEX_HOME/<name>.config.toml`),
/// otherwise the OS home directory joined with `.codex` -- confirmed
/// live: rollout files land at `~/.codex/sessions/...` with `CODEX_HOME`
/// unset, the same default-fallback shape `gemini_home_dir`
/// (`crate::gemini`) uses for `GEMINI_CLI_HOME`.
fn codex_home_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        return Some(std::path::PathBuf::from(dir));
    }
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }?;
    Some(std::path::PathBuf::from(home).join(".codex"))
}

/// How many of the most recent day-directories under `sessions/` to
/// check before giving up. A session worth forking should have started
/// reasonably recently; this bounds the scan rather than walking a
/// `CODEX_HOME` that may hold months or years of history.
const MAX_DAYS_TO_SCAN: usize = 30;

/// Locates the source session's own native Codex thread id by scanning
/// `<codex_home>/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl` for the file
/// whose first line's `SessionMeta.cwd` matches `workspace_cwd` exactly.
///
/// Each `(YYYY, MM, DD)` directory that actually exists is checked
/// newest-first -- plain lexicographic sort on the zero-padded path
/// components sorts chronologically for free, no date arithmetic needed
/// -- capped at [`MAX_DAYS_TO_SCAN`]. `None` if nothing matches within
/// that window, or on any I/O error along the way; every case means the
/// same thing to a caller: nothing to fork from right now.
fn locate_native_thread_id(
    codex_home: &std::path::Path,
    workspace_cwd: &std::path::Path,
) -> Option<String> {
    let mut day_dirs = day_directories(&codex_home.join("sessions"));
    day_dirs.sort_by(|a, b| b.cmp(a));
    let target_cwd = workspace_cwd.to_string_lossy().into_owned();

    for day_dir in day_dirs.into_iter().take(MAX_DAYS_TO_SCAN) {
        let Ok(entries) = std::fs::read_dir(&day_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_candidate = entry.file_type().is_ok_and(|t| t.is_file())
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"));
            if !is_candidate {
                continue;
            }
            if let Some(id) = session_meta_id_if_cwd_matches(&path, &target_cwd) {
                return Some(id);
            }
        }
    }
    None
}

/// Every existing `<sessions_dir>/<YYYY>/<MM>/<DD>` path, unsorted.
fn day_directories(sessions_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut days = Vec::new();
    let Ok(years) = std::fs::read_dir(sessions_dir) else {
        return days;
    };
    for year in years.flatten() {
        let Ok(months) = std::fs::read_dir(year.path()) else {
            continue;
        };
        for month in months.flatten() {
            let Ok(day_entries) = std::fs::read_dir(month.path()) else {
                continue;
            };
            for day in day_entries.flatten() {
                if day.file_type().is_ok_and(|t| t.is_dir()) {
                    days.push(day.path());
                }
            }
        }
    }
    days
}

/// Reads `path`'s first line and, if it is a `SessionMeta` record whose
/// own `cwd` matches `target_cwd` exactly, returns its `id`. `None` on
/// any shape mismatch, parse failure, or I/O error -- a substring/full
/// parse hybrid isn't needed here the way Gemini's `first_line_is_main_session`
/// uses one, since this also needs to *extract* a field, not just
/// classify the line.
fn session_meta_id_if_cwd_matches(path: &std::path::Path, target_cwd: &str) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut first_line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(file), &mut first_line).ok()?;
    let record: serde_json::Value = serde_json::from_str(&first_line).ok()?;
    if record.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = record.get("payload")?;
    if payload.get("cwd")?.as_str()? != target_cwd {
        return None;
    }
    payload.get("id")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_bare_is_just_the_program() {
        assert_eq!(
            Codex.launch_args(&[], false, None),
            vec!["codex".to_owned()]
        );
    }

    #[test]
    fn launch_args_passes_through_an_initial_prompt() {
        assert_eq!(
            Codex.launch_args(&["fix the failing test".to_owned()], false, None),
            vec!["codex".to_owned(), "fix the failing test".to_owned()]
        );
    }

    #[test]
    fn launch_args_adds_bypass_and_sandbox_flags_when_hooks_enabled() {
        assert_eq!(
            Codex.launch_args(&[], true, None),
            vec![
                "codex".to_owned(),
                "--dangerously-bypass-hook-trust".to_owned(),
                "--sandbox".to_owned(),
                "danger-full-access".to_owned(),
            ]
        );
    }

    #[test]
    fn launch_args_ignores_a_native_id_it_cannot_pin() {
        assert_eq!(
            Codex.launch_args(&[], false, Some("some-id")),
            Codex.launch_args(&[], false, None)
        );
    }

    #[test]
    fn supports_fork_is_true() {
        // The mechanism is real (see `fork_args`'s own docs) even though
        // any single call can still come back `None` for a call-specific
        // reason -- see `locate_native_thread_id`'s own tests below.
        assert!(Codex.supports_fork());
    }

    /// A scratch stand-in for `codex_home_dir()`'s own directory, removed
    /// on drop. Not a process-wide env var override (which would race
    /// every other test in this binary running concurrently) --
    /// `locate_native_thread_id` takes its `codex_home` as a plain
    /// argument, so these tests just build one and pass it directly. See
    /// `gemini.rs`'s own `ScratchGeminiHome` for the same pattern.
    struct ScratchCodexHome(std::path::PathBuf);

    impl ScratchCodexHome {
        fn new(label: &str) -> Self {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (label, std::process::id(), line!()).hash(&mut hasher);
            let dir = std::env::temp_dir().join(format!("smch{:x}", hasher.finish()));
            std::fs::create_dir_all(&dir).expect("create the scratch codex home");
            ScratchCodexHome(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }

        /// Writes a rollout file at `sessions/<y>/<m>/<d>/rollout-<filename_suffix>.jsonl`,
        /// with a first line shaped like a real `SessionMeta` record.
        fn write_rollout(
            &self,
            y: &str,
            m: &str,
            d: &str,
            filename_suffix: &str,
            cwd: &std::path::Path,
            id: &str,
        ) {
            let dir = self.0.join("sessions").join(y).join(m).join(d);
            std::fs::create_dir_all(&dir).expect("create the day directory");
            let content = format!(
                r#"{{"timestamp":"2026-01-01T00:00:00.000Z","type":"session_meta","payload":{{"session_id":"{id}","id":"{id}","cwd":"{cwd}","originator":"codex_exec"}}}}"#,
                cwd = cwd.to_string_lossy().replace('\\', "\\\\"),
            );
            std::fs::write(
                dir.join(format!("rollout-{filename_suffix}.jsonl")),
                content,
            )
            .expect("write rollout file");
        }
    }

    impl Drop for ScratchCodexHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn locate_native_thread_id_returns_none_with_no_sessions_dir_at_all() {
        let home = ScratchCodexHome::new("no-sessions-dir");
        let workspace = std::path::Path::new("/some/project");
        assert_eq!(locate_native_thread_id(home.path(), workspace), None);
    }

    #[test]
    fn locate_native_thread_id_returns_none_without_a_matching_cwd() {
        let home = ScratchCodexHome::new("no-match");
        let workspace = std::path::Path::new("/some/project");
        home.write_rollout(
            "2026",
            "08",
            "17",
            "2026-08-17T16-36-16-aaaaaaaa",
            std::path::Path::new("/some/other/project"),
            "aaaaaaaa-0000-0000-0000-000000000000",
        );
        assert_eq!(locate_native_thread_id(home.path(), workspace), None);
    }

    #[test]
    fn locate_native_thread_id_finds_a_matching_rollout_file() {
        let home = ScratchCodexHome::new("finds-match");
        let workspace = std::path::Path::new("/some/project");
        home.write_rollout(
            "2026",
            "08",
            "17",
            "2026-08-17T16-36-16-bbbbbbbb",
            workspace,
            "bbbbbbbb-1111-1111-1111-111111111111",
        );
        assert_eq!(
            locate_native_thread_id(home.path(), workspace),
            Some("bbbbbbbb-1111-1111-1111-111111111111".to_owned())
        );
    }

    #[test]
    fn locate_native_thread_id_checks_the_newest_day_directory_first() {
        let home = ScratchCodexHome::new("newest-day-first");
        let workspace = std::path::Path::new("/some/project");
        // An older rollout for a *different* workspace, plus a newer
        // rollout for the one actually being searched for -- if the scan
        // did not prefer the newest day, or scanned in the wrong order,
        // this would still pass by accident unless the older file could
        // somehow shadow the newer one. It can't: only one file matches
        // `workspace` at all, on the *older* day here, deliberately, to
        // prove the scan doesn't stop at the newest day when nothing
        // there matches.
        home.write_rollout(
            "2026",
            "08",
            "16",
            "2026-08-16T10-00-00-cccccccc",
            workspace,
            "cccccccc-2222-2222-2222-222222222222",
        );
        home.write_rollout(
            "2026",
            "08",
            "17",
            "2026-08-17T10-00-00-dddddddd",
            std::path::Path::new("/some/other/project"),
            "dddddddd-3333-3333-3333-333333333333",
        );
        assert_eq!(
            locate_native_thread_id(home.path(), workspace),
            Some("cccccccc-2222-2222-2222-222222222222".to_owned())
        );
    }

    #[test]
    fn locate_native_thread_id_ignores_non_rollout_files() {
        let home = ScratchCodexHome::new("ignore-non-rollout");
        let workspace = std::path::Path::new("/some/project");
        let dir = home.path().join("sessions/2026/08/17");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("not-a-rollout-file.jsonl"),
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"x","cwd":"{}"}}}}"#,
                workspace.display()
            ),
        )
        .unwrap();
        assert_eq!(locate_native_thread_id(home.path(), workspace), None);
    }

    #[test]
    fn hook_config_writes_codex_config_toml_with_every_event() {
        let id = sessionmgr_core::SessionId::new(1_700_000_000_000, 1);
        let (path, content) = Codex.hook_config(std::path::Path::new("C:/x/sessionmgr.exe"), &id);
        assert_eq!(path, std::path::PathBuf::from(".codex").join("config.toml"));
        assert!(content.contains("hooks = true"));
        for event in HOOK_EVENTS {
            assert!(
                content.contains(&format!("[[hooks.{event}]]")),
                "missing table for {event}:\n{content}"
            );
            assert!(content.contains(&format!(
                "'C:/x/sessionmgr.exe __hook-fire --session-id {id} --event {event}'"
            )));
        }
    }

    #[test]
    fn hook_signal_maps_needs_input_events() {
        assert_eq!(
            Codex.hook_signal("PermissionRequest"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
        assert_eq!(
            Codex.hook_signal("Stop"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
        assert_eq!(Codex.hook_signal("SubagentStop"), HookOutcome::Notify);
        assert_eq!(Codex.hook_signal("PreToolUse"), HookOutcome::Ignore);
    }

    /// Real, captured (vt100-rendered) folder-trust gate.
    const TRUST_SCREEN: &str = "\
> You are in C:\\adaptertest\\repo\\.sessionmgr-worktrees\\m06k9vpzn1hm

  Note: You're in a subdirectory of a Git project. Trusting will apply to the
  repository root: C:\\adaptertest\\repo

  Do you trust the contents of this directory? Working with untrusted contents
  comes with higher risk of prompt injection. Trusting the directory allows
  project-local config, hooks, and exec policies to load.

  1. Yes, continue
  2. No, quit

  Press enter to continue";

    /// Real, captured plugin-hooks review screen (triggered by
    /// marketplace-bundled hooks needing review before Codex will run
    /// them).
    const HOOK_REVIEW_SCREEN: &str = "\
  Hooks
  Lifecycle hooks from config and enabled plugins.
  \u{26a0} 2 hooks need review before they can run.

  Event           Installed   Active      Review
  PreToolUse      0           0           0

  Press t to trust all; enter to review hooks; esc to close";

    #[test]
    fn trust_gate_needs_input() {
        assert_eq!(Codex.needs_input(TRUST_SCREEN), AgentSignal::NeedsInput);
    }

    #[test]
    fn hook_review_screen_needs_input() {
        assert_eq!(
            Codex.needs_input(HOOK_REVIEW_SCREEN),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn plain_output_with_no_prompt_is_running() {
        assert_eq!(
            Codex.needs_input("just some ordinary output\nno prompt here"),
            AgentSignal::Running
        );
    }

    #[test]
    fn has_verified_hooks_is_true() {
        assert!(Codex.has_verified_hooks());
    }
}
