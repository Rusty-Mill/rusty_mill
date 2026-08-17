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

use sessionmgr_core::ports::{AgentAdapterPort, AgentSignal};

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

impl AgentAdapterPort for Codex {
    fn launch_args(&self, extra: &[String]) -> Vec<String> {
        let mut args = vec!["codex".to_owned()];
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_bare_is_just_the_program() {
        assert_eq!(Codex.launch_args(&[]), vec!["codex".to_owned()]);
    }

    #[test]
    fn launch_args_passes_through_an_initial_prompt() {
        assert_eq!(
            Codex.launch_args(&["fix the failing test".to_owned()]),
            vec!["codex".to_owned(), "fix the failing test".to_owned()]
        );
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
