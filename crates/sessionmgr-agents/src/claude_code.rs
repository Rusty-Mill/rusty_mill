//! The Claude Code adapter.
//!
//! `needs_input`'s patterns are transcribed from real, measured output --
//! not assumed -- captured by running an actual `claude` session through
//! `sessionmgr` on this machine and rendering its transcript through
//! `vt100`. See `docs/phase-3-report.md` for the full capture method and
//! raw evidence.

use sessionmgr_core::ports::{AgentAdapterPort, AgentSignal, HookOutcome};

pub struct ClaudeCode;

/// Measured directly: Claude Code's bottom status bar reads
/// `⏸ manual mode on · esc to interrupt · ← for agents` while it is
/// actively working, and switches to `⏸ manual mode on · ? for
/// shortcuts · ← for agents` once it is idle at its main prompt again.
/// Checked first, unconditionally: nothing else in this module is
/// allowed to call a session `NeedsInput` while this is on screen.
const BUSY: &str = "esc to interrupt";

/// Real prompts this session has actually shown, each requiring the user
/// to answer before anything continues:
///
/// - `"Do you want to proceed?"` / `"requires approval"` / `"Esc to
///   cancel"` -- the tool-permission dialog (captured approving a `dir`
///   command).
/// - `"Quick safety check"` -- the first-run "do you trust this folder"
///   gate.
/// - `"? for shortcuts"` -- the ordinary idle marker in the bottom
///   status bar, present exactly when [`BUSY`] is not.
const NEEDS_INPUT_MARKERS: &[&str] = &[
    "Do you want to proceed?",
    "requires approval",
    "Esc to cancel",
    "Quick safety check",
    "? for shortcuts",
];

/// Events this adapter installs a hook for, and what each means.
///
/// `SessionStart` and `Stop` are live-verified (Phase 1's Spike A, and
/// again in Phase 4's own hook-install spike -- see
/// `docs/phase-4-hooks-report.md`). `Notification` is real, stable,
/// documented Anthropic behavior (fires on a permission request or a
/// 60-second idle timeout) but was not independently fired-and-observed
/// in this project's own testing -- recorded honestly rather than
/// folded into "verified." `SubagentStop` is Claude Code's own
/// documented event for a sub-agent finishing, matching PLAN.md's
/// `SubagentFinished` webhook category.
const HOOK_EVENTS: &[&str] = &["SessionStart", "Notification", "Stop", "SubagentStop"];

impl AgentAdapterPort for ClaudeCode {
    fn launch_args(
        &self,
        extra: &[String],
        _hooks_enabled: bool,
        native_id: Option<&str>,
    ) -> Vec<String> {
        // Measured: Claude Code's own hooks fired cleanly right after the
        // ordinary folder-trust gate, with no extra review screen and no
        // bypass flag needed -- unlike Codex. `hooks_enabled` genuinely
        // changes nothing here; it is still a parameter (not a
        // Claude-Code-only method signature) because the port is shared.
        let mut args = vec!["claude".to_owned()];
        // `--session-id <uuid>`, live-verified in ADR-0003's spike: a
        // real `claude --session-id <uuid> -p "..."` run produced a real
        // transcript at exactly that id. Pinning it at launch, always
        // (not only when a `--hooks`-style flag asks for it) is what
        // makes an *ordinary* session forkable later with zero extra
        // machinery -- no directory-scanning discovery step is needed
        // anywhere in this adapter or the worker that spawns it.
        if let Some(id) = native_id {
            args.push("--session-id".to_owned());
            args.push(id.to_owned());
        }
        args.extend(extra.iter().cloned());
        args
    }

    fn needs_input(&self, screen_text: &str) -> AgentSignal {
        if screen_text.contains(BUSY) {
            return AgentSignal::Running;
        }
        if NEEDS_INPUT_MARKERS.iter().any(|m| screen_text.contains(m)) {
            AgentSignal::NeedsInput
        } else {
            AgentSignal::Running
        }
    }

    fn has_verified_hooks(&self) -> bool {
        // Phase 1's Spike A: SessionStart and Stop hooks both fired for
        // real, launched detached and headless -- the exact shape this
        // project depends on. See docs/phase-1-report.md.
        true
    }

    fn hook_config(
        &self,
        hook_fire_exe: &std::path::Path,
        session_id: &sessionmgr_core::SessionId,
    ) -> (std::path::PathBuf, String) {
        let mut hooks = serde_json::Map::new();
        for event in HOOK_EVENTS {
            let command = format!(
                "{} __hook-fire --session-id {session_id} --event {event}",
                hook_fire_exe.display()
            );
            hooks.insert(
                (*event).to_owned(),
                serde_json::json!([{ "hooks": [{ "type": "command", "command": command }] }]),
            );
        }
        let content = serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks }))
            .expect("a Map<String, Value> of our own construction always serializes");
        (
            std::path::PathBuf::from(".claude").join("settings.json"),
            content,
        )
    }

    fn hook_signal(&self, event: &str) -> HookOutcome {
        match event {
            "Notification" | "Stop" => HookOutcome::Status(AgentSignal::NeedsInput),
            "SubagentStop" => HookOutcome::Notify,
            _ => HookOutcome::Ignore,
        }
    }

    fn supports_fork(&self) -> bool {
        true
    }

    fn fork_args(
        &self,
        source_native_id: &str,
        new_native_id: &str,
        extra: &[String],
    ) -> Option<Vec<String>> {
        // `--resume <id> --fork-session`, per `--help`'s own description:
        // "When resuming, create a new session ID instead of reusing the
        // original." Combined with `--session-id`, live-verified in
        // `docs/phase-6-report.md` to pin the *forked* session's own new
        // id too, rather than leaving it to whatever Claude Code would
        // have auto-assigned -- keeping every session this adapter
        // creates, forked or not, equally forkable again afterward.
        let mut args = vec![
            "claude".to_owned(),
            "--resume".to_owned(),
            source_native_id.to_owned(),
            "--fork-session".to_owned(),
            "--session-id".to_owned(),
            new_native_id.to_owned(),
        ];
        args.extend(extra.iter().cloned());
        Some(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_bare_is_just_the_program() {
        assert_eq!(
            ClaudeCode.launch_args(&[], false, None),
            vec!["claude".to_owned()]
        );
    }

    #[test]
    fn launch_args_passes_through_an_initial_prompt() {
        assert_eq!(
            ClaudeCode.launch_args(&["fix the failing test".to_owned()], false, None),
            vec!["claude".to_owned(), "fix the failing test".to_owned()]
        );
    }

    #[test]
    fn launch_args_unaffected_by_hooks_enabled() {
        // Documented, measured difference from Codex: Claude Code needed
        // no extra flag for its own hooks to fire.
        assert_eq!(
            ClaudeCode.launch_args(&[], true, None),
            ClaudeCode.launch_args(&[], false, None)
        );
    }

    #[test]
    fn launch_args_pins_the_native_session_id_when_given() {
        assert_eq!(
            ClaudeCode.launch_args(&[], false, Some("11111111-1111-1111-1111-111111111111")),
            vec![
                "claude".to_owned(),
                "--session-id".to_owned(),
                "11111111-1111-1111-1111-111111111111".to_owned(),
            ]
        );
    }

    #[test]
    fn fork_args_resumes_the_source_and_pins_the_forks_own_new_id() {
        assert!(ClaudeCode.supports_fork());
        let args = ClaudeCode
            .fork_args("source-id", "new-id", &["continue differently".to_owned()])
            .expect("Claude Code supports fork");
        assert_eq!(
            args,
            vec![
                "claude".to_owned(),
                "--resume".to_owned(),
                "source-id".to_owned(),
                "--fork-session".to_owned(),
                "--session-id".to_owned(),
                "new-id".to_owned(),
                "continue differently".to_owned(),
            ]
        );
    }

    #[test]
    fn hook_config_writes_claude_settings_json_with_every_event() {
        let id = sessionmgr_core::SessionId::new(1_700_000_000_000, 1);
        let (path, content) =
            ClaudeCode.hook_config(std::path::Path::new("C:/x/sessionmgr.exe"), &id);
        assert_eq!(
            path,
            std::path::PathBuf::from(".claude").join("settings.json")
        );
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        for event in HOOK_EVENTS {
            let command = parsed["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("missing command for {event}"));
            assert!(command.contains("C:/x/sessionmgr.exe"));
            assert!(command.contains(&format!("--session-id {id}")));
            assert!(command.contains(&format!("--event {event}")));
        }
    }

    #[test]
    fn hook_signal_maps_needs_input_events() {
        assert_eq!(
            ClaudeCode.hook_signal("Notification"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
        assert_eq!(
            ClaudeCode.hook_signal("Stop"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
        assert_eq!(ClaudeCode.hook_signal("SubagentStop"), HookOutcome::Notify);
        assert_eq!(ClaudeCode.hook_signal("PreToolUse"), HookOutcome::Ignore);
    }

    /// Real, captured (vt100-rendered) idle screen: the welcome box plus
    /// the bottom status bar showing `? for shortcuts`.
    const IDLE_SCREEN: &str = "\
╭─ Claude Code ──────────────────────────────────────────╮
│                                                        │
│                   Welcome back Nano!                   │
╰────────────────────────────────────────────────────────╯
❯
──────────────────────────────────────────────────────────
  ⏸ manual mode on · ? for shortcuts · ← for agents
                                        ● high · /effort";

    /// Real, captured busy screen: mid-turn, waiting on a tool call.
    const BUSY_SCREEN: &str = "\
❯ list the files in this directory using dir, then stop
──────────────────────────────────────────────────────────
  ⏸ manual mode on · esc to interrupt · ← for agents
✻ Whirlpooling… (2s · thinking with high effort)
● Bash(dir)
  ⎿ Waiting…";

    /// Real, captured permission dialog, shown mid-turn when a tool call
    /// needs approval.
    const APPROVAL_SCREEN: &str = "\
 Bash command
   dir
   List files in current directory
 This command requires approval
 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for: dir *
   3. No
 Esc to cancel · Tab to amend · ctrl+e to explain";

    /// Real, captured first-run trust gate.
    const TRUST_SCREEN: &str = "\
Quick safety check: Is this a project you created or one you trust?
❯ 1. Yes, I trust this folder
  2. No, exit
Enter to confirm · Esc to cancel";

    #[test]
    fn idle_screen_needs_input() {
        assert_eq!(ClaudeCode.needs_input(IDLE_SCREEN), AgentSignal::NeedsInput);
    }

    #[test]
    fn busy_screen_is_running_even_though_it_mentions_waiting() {
        // The exact case BUSY exists to guard: "Waiting…" appears in the
        // tool-call line, but "esc to interrupt" in the status bar is
        // what actually says the agent has not stopped.
        assert_eq!(ClaudeCode.needs_input(BUSY_SCREEN), AgentSignal::Running);
    }

    #[test]
    fn approval_dialog_needs_input() {
        assert_eq!(
            ClaudeCode.needs_input(APPROVAL_SCREEN),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn trust_gate_needs_input() {
        assert_eq!(
            ClaudeCode.needs_input(TRUST_SCREEN),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn plain_output_with_no_prompt_is_running() {
        assert_eq!(
            ClaudeCode.needs_input("just some ordinary output\nno prompt here"),
            AgentSignal::Running
        );
    }

    #[test]
    fn has_verified_hooks_is_true() {
        assert!(ClaudeCode.has_verified_hooks());
    }
}
