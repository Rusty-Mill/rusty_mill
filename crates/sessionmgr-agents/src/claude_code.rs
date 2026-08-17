//! The Claude Code adapter.
//!
//! `needs_input`'s patterns are transcribed from real, measured output --
//! not assumed -- captured by running an actual `claude` session through
//! `sessionmgr` on this machine and rendering its transcript through
//! `vt100`. See `docs/phase-3-report.md` for the full capture method and
//! raw evidence.

use sessionmgr_core::ports::{AgentAdapterPort, AgentSignal};

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

impl AgentAdapterPort for ClaudeCode {
    fn launch_args(&self, extra: &[String]) -> Vec<String> {
        let mut args = vec!["claude".to_owned()];
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_bare_is_just_the_program() {
        assert_eq!(ClaudeCode.launch_args(&[]), vec!["claude".to_owned()]);
    }

    #[test]
    fn launch_args_passes_through_an_initial_prompt() {
        assert_eq!(
            ClaudeCode.launch_args(&["fix the failing test".to_owned()]),
            vec!["claude".to_owned(), "fix the failing test".to_owned()]
        );
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
