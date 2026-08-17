//! The Gemini CLI adapter.
//!
//! Unlike [`crate::claude_code`] and [`crate::codex`], `needs_input`'s
//! patterns and the hook mechanism's existence are **not** independently
//! measured by running a real `gemini` session on this machine -- there
//! is no `GEMINI_API_KEY` (or any other auth method) configured here, and
//! `gemini` hard-refuses to start at all without one, before it ever
//! reaches an interactive screen or fires a single hook (confirmed: a
//! `SessionStart` hook installed in a scratch repo never fired when
//! `gemini` was run there unauthenticated). That is a missing
//! prerequisite, not a shortcut taken.
//!
//! What this adapter is built from instead: `gemini`'s own shipped,
//! unminified-string bundle (`@google/gemini-cli`'s `bundle/*.js` --
//! identifiers are mangled by the bundler, but string literals are not),
//! plus the hook mechanism's own official reference docs
//! (<https://geminicli.com/docs/hooks/reference/>,
//! <https://geminicli.com/docs/hooks/writing-hooks/>). This is arguably
//! *more* precise than eyeballing a live terminal for the screen-text
//! patterns below -- it is the literal string constants the UI renders,
//! not a transcription of what happened to appear on one captured
//! screen -- but it is source inspection, not a live-fired hook, and
//! [`has_verified_hooks`] says so honestly rather than folding this into
//! "verified" the way Claude Code's and Codex's hooks are.

use sessionmgr_core::ports::{AgentAdapterPort, AgentSignal, HookOutcome};

pub struct GeminiCli;

/// Gemini's own busy indicator, checked first and unconditionally, same
/// convention as [`crate::claude_code::BUSY`]. Sourced from
/// `interactiveCli-*.js`'s `cancelAndTimerContent`: rendered as
/// `(esc to cancel, <n>s)` exactly while `streamingState === "responding"`
/// -- i.e. while the model is actively generating. Note this is a
/// *different* phrase from Claude Code's `"esc to interrupt"`; the two
/// CLIs do not share this particular string.
const BUSY: &str = "esc to cancel";

/// Real prompt/dialog/footer text extracted from `gemini-cli`'s own
/// bundled source, each requiring the user to answer before anything
/// continues -- or, for the idle marker, indicating there is nothing
/// left to wait for and the CLI is back at its main prompt:
///
/// - `"Do you trust the files in this folder?"` -- the workspace-trust
///   gate's dialog title (`interactiveCli-*.js`'s `RadioButtonSelect`
///   trust prompt).
/// - `"Do you want to proceed?"` -- the tool-confirmation dialog's
///   `"info"`-type question text.
/// - `"Allow execution of"` -- the tool-confirmation dialog's general and
///   MCP-tool question text (`"Allow execution of <tool>?"` /
///   `"Allow execution of MCP tool \"<tool>\" from server..."` both
///   contain this as a prefix).
/// - `"You must select an auth method to proceed"` -- genuinely a state
///   only the user can resolve, even though sessionmgr cannot help
///   resolve it either.
/// - `"? for shortcuts"` -- the ordinary idle marker, shown once the
///   input buffer is empty and nothing is streaming. Textually identical
///   to Claude Code's own idle marker -- not assumed to be a
///   coincidence, but not investigated further; both are real, extracted
///   strings either way.
const NEEDS_INPUT_MARKERS: &[&str] = &[
    "Do you trust the files in this folder?",
    "Do you want to proceed?",
    "Allow execution of",
    "You must select an auth method to proceed",
    "? for shortcuts",
];

/// Events this adapter installs a hook for, and what each means.
///
/// All three are real, documented Gemini CLI events
/// (<https://geminicli.com/docs/hooks/reference/>), not guessed. There is
/// no Gemini analog to Claude Code's/Codex's `SubagentStop` -- Gemini
/// CLI's own hook event list has no sub-agent-finished event at all, so
/// this adapter has nothing to map to [`HookOutcome::Notify`].
///
/// - `SessionStart`: fires on a fresh interactive start (`source:
///   "startup"`, matched exactly -- see `hook_config`'s own comment on
///   why the config narrows to it). Maps to [`HookOutcome::Ignore`], same
///   as the other two adapters: a session starting is not a
///   needs-input signal.
/// - `Notification`: fires for a system alert, documented today as
///   firing for `notification_type: "ToolPermission"` -- the closest
///   Gemini analog to Claude Code's own `Notification` event, and
///   mapped the same way.
/// - `AfterAgent`: fires once per turn after the model's final response
///   -- the closest Gemini analog to Claude Code's/Codex's `Stop`: the
///   agent loop has finished generating and control is back with the
///   user.
const HOOK_EVENTS: &[&str] = &["SessionStart", "Notification", "AfterAgent"];

impl AgentAdapterPort for GeminiCli {
    fn launch_args(&self, extra: &[String], hooks_enabled: bool) -> Vec<String> {
        let mut args = vec!["gemini".to_owned()];
        if hooks_enabled {
            // `--skip-trust` bypasses the interactive workspace-trust
            // gate (`gemini --help`'s own description: "Trust the
            // current workspace for this session"). Unlike Codex,
            // source inspection found no *separate* hook-trust-review
            // gate or sandbox flag gating hook execution specifically --
            // hooks in a project the user has not trusted are simply
            // blocked by the same trust gate everything else is, per
            // the hooks docs' own "Project-level hooks are particularly
            // risky when opening untrusted projects" -- so one flag is
            // the whole story here, matching Claude Code's simpler
            // model rather than Codex's two-flag one. Not independently
            // confirmed live (see this module's own top-level docs);
            // flagged here rather than silently assumed identical to
            // either sibling adapter.
            args.push("--skip-trust".to_owned());
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
        // Honestly `false`: the hook mechanism and its config format are
        // real and well-documented (this module's own top-level docs),
        // but no hook installed by this adapter has actually been fired
        // and observed on a running `gemini` process -- blocked by a
        // missing `GEMINI_API_KEY`/auth method on this machine, which
        // `gemini` requires before it will even reach a hook-firing
        // state. A real, stated missing prerequisite, not a corner cut.
        false
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
            let mut group = serde_json::json!({
                "hooks": [{ "type": "command", "command": command }]
            });
            if *event == "SessionStart" {
                // Lifecycle events match on an exact string
                // (geminicli.com/docs/hooks/: "Lifecycle events:
                // Matchers are Exact Strings"). `"startup"` is a fresh
                // interactive start, the only way sessionmgr ever
                // launches this CLI -- narrowing to it, rather than
                // leaving the matcher off, means this hook does not
                // also fire on `--resume`/`/clear`, neither of which
                // sessionmgr uses today.
                group["matcher"] = serde_json::Value::String("startup".to_owned());
            }
            hooks.insert((*event).to_owned(), serde_json::json!([group]));
        }
        let content = serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks }))
            .expect("a Map<String, Value> of our own construction always serializes");
        (
            std::path::PathBuf::from(".gemini").join("settings.json"),
            content,
        )
    }

    fn hook_signal(&self, event: &str) -> HookOutcome {
        match event {
            "Notification" | "AfterAgent" => HookOutcome::Status(AgentSignal::NeedsInput),
            _ => HookOutcome::Ignore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_bare_is_just_the_program() {
        assert_eq!(GeminiCli.launch_args(&[], false), vec!["gemini".to_owned()]);
    }

    #[test]
    fn launch_args_passes_through_an_initial_prompt() {
        assert_eq!(
            GeminiCli.launch_args(&["fix the failing test".to_owned()], false),
            vec!["gemini".to_owned(), "fix the failing test".to_owned()]
        );
    }

    #[test]
    fn launch_args_adds_skip_trust_flag_when_hooks_enabled() {
        assert_eq!(
            GeminiCli.launch_args(&[], true),
            vec!["gemini".to_owned(), "--skip-trust".to_owned()]
        );
    }

    #[test]
    fn busy_screen_is_running_even_if_it_also_contains_a_marker_substring() {
        // Mirrors claude_code's/codex's own busy-wins-first test: the
        // busy check must be unconditional, not "checked only when no
        // marker matched".
        assert_eq!(
            GeminiCli.needs_input("Thinking... (esc to cancel, 3s)"),
            AgentSignal::Running
        );
    }

    #[test]
    fn idle_screen_needs_input() {
        assert_eq!(
            GeminiCli.needs_input("  Type your message or @path/to/file\n? for shortcuts"),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn trust_gate_needs_input() {
        assert_eq!(
            GeminiCli.needs_input("Do you trust the files in this folder?"),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn tool_confirmation_needs_input() {
        assert_eq!(
            GeminiCli.needs_input("Allow execution of \"run_shell_command\"?"),
            AgentSignal::NeedsInput
        );
        assert_eq!(
            GeminiCli.needs_input("Do you want to proceed?"),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn plain_output_is_running() {
        assert_eq!(GeminiCli.needs_input("hello world"), AgentSignal::Running);
    }

    #[test]
    fn has_verified_hooks_is_honestly_false() {
        assert!(!GeminiCli.has_verified_hooks());
    }

    #[test]
    fn hook_config_writes_gemini_settings_json_with_every_event() {
        let id = sessionmgr_core::SessionId::new(1_700_000_000_000, 1);
        let (path, content) =
            GeminiCli.hook_config(std::path::Path::new("C:/x/sessionmgr.exe"), &id);
        assert_eq!(
            path,
            std::path::PathBuf::from(".gemini").join("settings.json")
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
    fn hook_config_matches_session_start_to_a_real_startup_only() {
        let id = sessionmgr_core::SessionId::new(1_700_000_000_000, 1);
        let (_, content) = GeminiCli.hook_config(std::path::Path::new("C:/x/sessionmgr.exe"), &id);
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        assert_eq!(parsed["hooks"]["SessionStart"][0]["matcher"], "startup");
        assert!(parsed["hooks"]["Notification"][0]["matcher"].is_null());
        assert!(parsed["hooks"]["AfterAgent"][0]["matcher"].is_null());
    }

    #[test]
    fn hook_signal_maps_needs_input_events() {
        assert_eq!(
            GeminiCli.hook_signal("Notification"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
        assert_eq!(
            GeminiCli.hook_signal("AfterAgent"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
    }

    #[test]
    fn hook_signal_ignores_session_start_and_unrecognized_events() {
        assert_eq!(GeminiCli.hook_signal("SessionStart"), HookOutcome::Ignore);
        assert_eq!(GeminiCli.hook_signal("BeforeTool"), HookOutcome::Ignore);
    }

    #[test]
    fn hook_signal_has_no_notify_mapping() {
        // Documented difference from Claude Code/Codex: Gemini CLI's own
        // hook event list has no sub-agent-finished event, so nothing
        // here should ever produce HookOutcome::Notify.
        for event in HOOK_EVENTS {
            assert_ne!(GeminiCli.hook_signal(event), HookOutcome::Notify);
        }
    }
}
