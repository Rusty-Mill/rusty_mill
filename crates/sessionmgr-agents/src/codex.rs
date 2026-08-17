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

use sessionmgr_core::ports::{AgentAdapterPort, AgentSignal, HookOutcome};

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

    /// Not supported here, deliberately -- not a gap silently left open.
    ///
    /// `codex fork <id>` is real (confirmed via Codex's own test suite,
    /// `docs/decisions/0003-resume-fork-spike.md`), but reaching it needs
    /// this session's *native* Codex thread id, and Codex has no launch
    /// flag to let a caller pin one the way Claude Code's `--session-id`
    /// does -- confirmed absent from a real `codex --help`. The id is
    /// always self-assigned and would have to be *discovered* after the
    /// fact (Codex's own rollout files embed it in their filename,
    /// `rollout-<timestamp>-<thread-id>.jsonl`, which is a workable
    /// mechanism but a genuinely separate piece of machinery -- a
    /// post-spawn filesystem watch, not a pure format-producing method
    /// like this trait's other ones). Building that blind, with no
    /// credentials in any environment available to verify it against a
    /// real `codex` process, is exactly the kind of unverified guess this
    /// project's own conventions ask not to ship. See
    /// `docs/phase-6-report.md` for the filed follow-up.
    fn fork_args(
        &self,
        _source_native_id: &str,
        _new_native_id: &str,
        _extra: &[String],
    ) -> Option<Vec<String>> {
        None
    }

    fn supports_fork(&self) -> bool {
        false
    }
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
    fn fork_is_not_supported() {
        assert!(!Codex.supports_fork());
        assert_eq!(Codex.fork_args("source", "new", &[]), None);
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
