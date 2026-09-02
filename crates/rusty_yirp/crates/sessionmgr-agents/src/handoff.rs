//! Turns a session's own raw transcript into plain text another agent CLI
//! can read as its opening context -- the mechanism behind Phase 7's
//! switch-agent-mid-session.
//!
//! **Deliberately not a per-CLI schema translation.** ADR-0003 proved
//! every supported CLI accepts *some* externally-supplied prior state,
//! but only within its own on-disk format; there is no shared schema to
//! translate between Claude Code's Messages-API-shaped JSONL, Codex's
//! `RolloutItem` enum, and Gemini's `{type, content}` records without
//! this crate taking on maintaining three schema parsers indefinitely
//! (see `docs/decisions/0003-resume-fork-spike.md`'s own consequences
//! section). Instead, the target agent gets the source session's
//! rendered transcript as its own **initial prompt** -- an ordinary
//! `extra` argument every adapter already accepts -- worded honestly as
//! a handoff, not implied to be a native resume. See
//! `docs/phase-7-report.md`.

/// Renders `transcript`'s raw PTY output bytes -- the concatenated
/// contents of a session's `transcript.jsonl` `Output` events, in order
/// -- as scrollback-complete plain text, and wraps it with a short
/// preamble telling the receiving agent what it is looking at.
///
/// Uses `vt100`, the same "interpret, don't print" engine
/// `pattern_watch::ScreenWatcher` uses for `needs_input`, for the same
/// reason: raw ANSI-stripping runs cursor-positioned text together with
/// no separating whitespace (`pattern_watch`'s own module docs have the
/// measured example). Unlike `ScreenWatcher`, which only ever needs the
/// *current* screen, this renders a virtual terminal tall enough to
/// hold the whole conversation rather than a fixed-size viewport with
/// scrollback -- there is no cheap way to read "everything ever
/// printed" back out of `vt100`'s own scrollback buffer in one pass, and
/// a screen this tall is simpler and just as correct for a one-shot
/// render than juggling scroll offsets.
///
/// `transcript` is capped to its **last** `MAX_INPUT_BYTES` before
/// rendering, and the virtual screen is capped to `MAX_ROWS` lines --
/// both deliberately, not an accidental `vt100` side effect: older
/// history is dropped in favor of the most recent, most relevant
/// context, matching PLAN.md's own sanctioned fallback for exactly this
/// situation ("a summarized system-prompt injection from the prior
/// transcript, not implied to preserve full state").
pub fn render_handoff(source_agent_label: &str, transcript: &[u8]) -> String {
    let tail = if transcript.len() > MAX_INPUT_BYTES {
        &transcript[transcript.len() - MAX_INPUT_BYTES..]
    } else {
        transcript
    };
    let mut parser = vt100::Parser::new(MAX_ROWS, COLS, 0);
    parser.process(tail);
    let rendered = parser.screen().contents();

    format!(
        "{PREAMBLE_1} {source_agent_label}{PREAMBLE_2}\n\n\
         --- prior conversation transcript (most recent {MAX_ROWS} lines) ---\n\
         {rendered}\n\
         --- end of prior conversation transcript ---\n\n\
         {CONTINUATION}"
    )
}

const MAX_INPUT_BYTES: usize = 512 * 1024;
const MAX_ROWS: u16 = 4000;
const COLS: u16 = 120;

const PREAMBLE_1: &str =
    "You are taking over an in-progress coding session that was previously being handled by";
const PREAMBLE_2: &str = ", a different AI coding assistant. Below is that assistant's own \
                           terminal transcript, rendered as plain text -- it is a record of \
                           what already happened, not something you produced.";
const CONTINUATION: &str = "Continue the task from here.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_plain_text_and_names_the_source_agent() {
        let out = render_handoff("Claude Code", b"hello from the prior session");
        assert!(out.contains("Claude Code"));
        assert!(out.contains("hello from the prior session"));
        assert!(out.contains("Continue the task from here."));
    }

    #[test]
    fn cursor_positioned_text_renders_with_real_spaces_not_run_together() {
        // The same defect `pattern_watch::ScreenWatcher` guards against:
        // a naive ANSI-strip-then-concatenate would read this back
        // wrong.
        let out = render_handoff("Claude Code", b"Welcome\x1b[1;5Hback\x1b[1;10HNano!");
        assert!(out.contains("back"));
        assert!(out.contains("Nano"));
    }

    #[test]
    fn a_transcript_longer_than_the_cap_still_renders_only_its_most_recent_bytes() {
        let mut long = vec![b'x'; MAX_INPUT_BYTES + 1000];
        long.extend_from_slice(b"CODEWORD-AT-THE-END");
        let out = render_handoff("Codex", &long);
        assert!(out.contains("CODEWORD-AT-THE-END"));
    }

    #[test]
    fn empty_transcript_still_produces_a_well_formed_handoff() {
        let out = render_handoff("Gemini CLI", b"");
        assert!(out.contains("Gemini CLI"));
        assert!(out.contains("Continue the task from here."));
    }
}
