//! `rusty_lines` — a hand-rolled line editor: a readline alternative with
//! no dependency on readline, ncurses, or any editing crate. Grown inside
//! the [rush shell](https://github.com/baileyrd/rush) as its `rustyline`
//! replacement, then extracted; its feature set is audited against the
//! other line editors in the wild — GNU readline, zsh ZLE, fish, ksh93's
//! emacs/vi modes, libedit, linenoise, replxx, rustyline, reedline — see
//! the README for the survey and the documented narrowings.
//!
//! The host integrates through the [`Hooks`] trait (completion, hints,
//! syntax highlighting, abbreviations, a live vi-mode flag, external
//! editor resolution, an interrupted-read callback, and host-command
//! key bindings); [`NoHooks`] gives plain editing.
//!
//! Layers, bottom to top:
//!   * raw terminal mode (termios) behind an RAII guard, so every exit path
//!     — including panics — restores the terminal; bracketed-paste mode
//!     behind a second guard, so a paste arrives as one event instead of a
//!     stream of fake keystrokes;
//!   * key decoding: UTF-8 assembly plus escape-sequence parsing (CSI/SS3
//!     with modifier parameters — Ctrl/Alt-arrows — Alt- chords, and the
//!     bracketed-paste envelope), with a short poll to tell a lone ESC
//!     from a sequence;
//!   * a render engine that repaints the whole edit region — never a
//!     diff, so cursor math stays exact — overwriting in place and
//!     clearing only the leftover tail (no flicker); repaints coalesce
//!     while more input is already queued (readline's trick, capped so
//!     a large flood still shows progress), so a keystroke isn't always
//!     synchronously followed by a paint. Display-width math (via
//!     `unicode_width`, aware of both CSI and OSC escapes), a
//!     readline-style `^X` visualization for control characters in the
//!     buffer, soft-wrap row accounting, forced wraps at exact column
//!     boundaries (avoiding the delayed-wrap ambiguity), multi-line
//!     prompts (prefix lines print once per region), syntax highlighting
//!     of the raw buffer, the dimmed history hint, and a right-side
//!     prompt (zsh's `$RPS1`), shown while the first row has room for it;
//!   * keymaps: the emacs set (kill ring with yank/yank-pop, undo,
//!     word-wise motion/kill/case/transpose, insert-last-argument,
//!     operate-and-get-next, quoted-insert, edit-in-`$EDITOR`) by
//!     default, plus a vi mode with counts, the `d`/`c`/`y` operators
//!     over motions (including `%` bracket matching and the `iw`/`aw`
//!     text objects), `f F t T ; ,` character finds, `G` history fetch,
//!     and the standard normal-mode edits — selected live per
//!     `read_line` via [`Hooks::vi_mode`], so switching needs no editor
//!     rebuild at all; single keys are rebindable to named
//!     [`EditorAction`]s or host commands ([`Editor::bind`],
//!     [`Editor::bind_host`] — readline's `bind`, bash's `bind -x`);
//!   * history: in-memory with consecutive-dedup (multi-line entries
//!     stored bash-style with `; ` joining; optional erase-dups and
//!     ignore-space), plain-file persistence, Up/Down navigation with
//!     draft preservation, Ctrl-R/Ctrl-S incremental search in both
//!     directions (with readline's failed-search state), and prefix
//!     search (PageUp/PageDown, Alt-p/Alt-n);
//!   * completion (Tab: longest-common-prefix insertion, then a sorted
//!     column-major candidate list, big ones behind readline's
//!     `Display all N possibilities?` question) and abbreviation
//!     expansion on space, both driven by the host's [`Hooks`].
//!
//! # Example
//!
//! ```no_run
//! use rusty_lines::{Editor, NoHooks, ReadResult};
//!
//! # fn main() -> std::io::Result<()> {
//! let mut ed = Editor::new();
//! match ed.read_line("prompt> ", "", &NoHooks)? {
//!     ReadResult::Line(line) => ed.add_history_entry(&line),
//!     ReadResult::Interrupted => { /* Ctrl-C */ }
//!     ReadResult::Eof => { /* Ctrl-D on an empty line */ }
//!     ReadResult::TimedOut => { /* read_line_timeout deadline only */ }
//! }
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

use std::collections::VecDeque;
use std::io::{self, Read, Write};

// Terminal syscall backend: the `libc` crate by default, `rusty_libc` under
// the `rusty-libc` feature, on Unix; `rusty_win32` on Windows.
mod term_sys;

/// One completion candidate: the text shown in the columned list, and the
/// text inserted into the buffer.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The text shown in the candidate list.
    pub display: String,
    /// The text inserted into the buffer when this candidate is chosen.
    pub replacement: String,
}

/// The host application's integration points. Every method has a no-op
/// default, so `&NoHooks` gives plain line editing; a shell implements
/// the lot (completion, hints, highlighting, abbreviations, a live
/// vi-mode flag, its own `$EDITOR` resolution, and a callback for
/// signals that interrupt the blocking read).
pub trait Hooks {
    /// Candidates for the word at `pos`, plus the byte offset that word
    /// starts at (the editor replaces `start..pos`).
    fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
        (0, Vec::new())
    }
    /// The dimmed inline suggestion shown after the buffer (fish-style);
    /// Right/End at end-of-line accepts it.
    fn hint(&self, _line: &str, _history: &[String]) -> Option<String> {
        None
    }
    /// Syntax highlighting: `line` is the *raw* buffer text, exactly what
    /// Enter would return — so a parser sees real tabs and newlines at
    /// their true byte offsets. Return it with ANSI SGR sequences
    /// (`ESC[…m`) inserted and the text itself unchanged; the editor
    /// re-applies its control-character visualization (`^X`, `⏎`, tab
    /// expansion) around the SGR markup afterwards, and widths are
    /// computed ANSI-aware. A line containing a literal ESC character is
    /// painted unhighlighted (the markup would be ambiguous).
    fn highlight(&self, line: &str) -> String {
        line.to_string()
    }
    /// Called when space is typed: return `Some((start, replacement))`
    /// to rewrite `start..cursor` first (fish-style abbreviations).
    fn expand_abbreviation(&self, _line: &str, _cursor: usize) -> Option<(usize, String)> {
        None
    }
    /// Checked at the start of every `read_line`: true selects the vi
    /// keymap, so a `set -o vi` needs no editor rebuild.
    fn vi_mode(&self) -> bool {
        false
    }
    /// The command for C-x C-e / vi `v` (edit the line in an editor).
    /// `None` falls back to `$VISUAL`, `$EDITOR`, then `vi`.
    fn external_editor(&self) -> Option<String> {
        None
    }
    /// Called when a signal interrupts the blocking read (`EINTR`), and
    /// also on every ~200ms idle tick while sitting at the prompt with
    /// nothing typed — a shell fires its pending traps here either way.
    /// The read then resumes. A hook that wants to print something of its
    /// own from here (not just check state) — e.g. a background job's
    /// completion notice — must call [`prepare_external_output`] first,
    /// every time, before writing anything: printing without it corrupts
    /// the editor's next repaint, since `render`'s own paint logic assumes
    /// the cursor is exactly where it left it.
    fn on_interrupted_read(&self) {}
    /// Invoked for a key bound via [`Editor::bind_host`] — bash's
    /// `bind -x` contract. The editor suspends raw mode, hands over the
    /// current buffer and cursor (byte offset) for the host to run its
    /// command with (`READLINE_LINE`/`READLINE_POINT`), writes back
    /// whatever the host left in them, and repaints. A cursor written
    /// past the end of the line (or inside a UTF-8 sequence) is clamped.
    fn host_binding(&self, _tag: &str, _line: &mut String, _cursor: &mut usize) {}
}

/// A no-op `Hooks`: plain editing with no completion, hints,
/// highlighting, or abbreviations.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHooks;
impl Hooks for NoHooks {}

/// A named edit action — every command in the emacs (and vi-insert)
/// keymap, using readline's command names camel-cased. What
/// [`Editor::bind`] binds a key to and [`Editor::bindings`] reports
/// (a shell's `bind '"\C-x": kill-line'` / `bind -P`).
///
/// The enum is `#[non_exhaustive]`: actions may be added, so downstream
/// name→action tables should match by name with a fallthrough.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorAction {
    /// Insert the typed character (with abbreviation expansion on space).
    SelfInsert,
    /// Finish the line and return it (`accept-line`, Enter).
    AcceptLine,
    /// Accept the line and pre-load the next `read_line` with the history
    /// entry after the one just executed (`operate-and-get-next`, C-o) —
    /// readline's replay-a-sequence-from-history command.
    OperateAndGetNext,
    /// Abandon the line ([`ReadResult::Interrupted`], C-c).
    Interrupt,
    /// Delete the character under the cursor; on an empty line, EOF
    /// (`delete-char` with bash's C-d end-of-file behavior).
    DeleteCharOrEof,
    /// Delete the character under the cursor (`delete-char`, Delete).
    DeleteChar,
    /// Delete the character before the cursor (`backward-delete-char`).
    BackwardDeleteChar,
    /// Move to the start of the line (`beginning-of-line`, C-a/Home).
    BeginningOfLine,
    /// Move to the end of the line; at end, accept the hint
    /// (`end-of-line`, C-e/End).
    EndOfLine,
    /// Move forward one character; at end, accept the hint
    /// (`forward-char`, C-f/Right).
    ForwardChar,
    /// Move back one character (`backward-char`, C-b/Left).
    BackwardChar,
    /// Move forward one alphanumeric word; at end, accept one hint word
    /// (`forward-word`, M-f/Ctrl-Right).
    ForwardWord,
    /// Move back one alphanumeric word (`backward-word`, M-b/Ctrl-Left).
    BackwardWord,
    /// Kill to the end of the line (`kill-line`, C-k).
    KillLine,
    /// Kill the entire line into the ring, wherever the cursor is
    /// (`kill-whole-line`; unbound by default, like readline).
    KillWholeLine,
    /// Delete the spaces and tabs around the cursor, without killing
    /// (`delete-horizontal-space`, M-\).
    DeleteHorizontalSpace,
    /// Kill to the start of the line (`unix-line-discard`, C-u).
    UnixLineDiscard,
    /// Kill the whitespace-delimited word before the cursor
    /// (`unix-word-rubout`, C-w).
    UnixWordRubout,
    /// Kill the alphanumeric word after the cursor (`kill-word`, M-d).
    KillWord,
    /// Kill the alphanumeric word before the cursor
    /// (`backward-kill-word`, M-Backspace).
    BackwardKillWord,
    /// Insert the top of the kill ring (`yank`, C-y).
    Yank,
    /// Rotate the last yank to the previous ring entry (`yank-pop`, M-y).
    YankPop,
    /// Transpose the characters around the cursor (`transpose-chars`, C-t).
    TransposeChars,
    /// Transpose the words around the cursor (`transpose-words`, M-t).
    TransposeWords,
    /// Uppercase to the end of the word (`upcase-word`, M-u).
    UpcaseWord,
    /// Lowercase to the end of the word (`downcase-word`, M-l).
    DowncaseWord,
    /// Capitalize the word (`capitalize-word`, M-c).
    CapitalizeWord,
    /// Undo the last edit (`undo`, C-_/C-z).
    Undo,
    /// Undo every edit to this line at once (`revert-line`, M-r).
    RevertLine,
    /// Insert the last word of the previous history entry; repeats cycle
    /// older entries (`insert-last-argument`, M-./M-_).
    InsertLastArgument,
    /// Recall the previous history entry (`previous-history`, Up/C-p).
    PreviousHistory,
    /// Recall the next history entry, or the stashed draft
    /// (`next-history`, Down/C-n).
    NextHistory,
    /// Jump to the oldest history entry (`beginning-of-history`, M-<).
    BeginningOfHistory,
    /// Back to the live draft line (`end-of-history`, M->).
    EndOfHistory,
    /// Previous history entry with the prefix before the cursor
    /// (`history-search-backward`, PageUp/M-p).
    HistorySearchBackward,
    /// Next prefix match, or back to the draft
    /// (`history-search-forward`, PageDown/M-n).
    HistorySearchForward,
    /// Enter incremental search, older matches
    /// (`reverse-search-history`, C-r).
    ReverseSearchHistory,
    /// Enter incremental search, newer matches
    /// (`forward-search-history`, C-s).
    ForwardSearchHistory,
    /// Clear the screen and repaint the line at the top
    /// (`clear-screen`, C-l).
    ClearScreen,
    /// Complete the word at the cursor: LCP insertion, candidate list,
    /// then menu cycling (`complete`, Tab). Behaves as [`MenuComplete`]
    /// under [`Editor::set_menu_complete`].
    ///
    /// [`MenuComplete`]: EditorAction::MenuComplete
    Complete,
    /// Insert the first completion candidate immediately; repeats cycle
    /// through the rest (`menu-complete`).
    MenuComplete,
    /// Cycle the completion candidates backward — starting from the last
    /// (`menu-complete-backward`, Shift-Tab; zsh `reverse-menu-complete`).
    MenuCompleteBackward,
    /// List the completion candidates without changing the buffer
    /// (`possible-completions`, M-?).
    PossibleCompletions,
    /// Insert every completion candidate, space-separated
    /// (`insert-completions`, M-*).
    InsertCompletions,
    /// Read one character and move the cursor to its next occurrence
    /// (`character-search`, C-]).
    CharacterSearch,
    /// Read one character and move the cursor back to its previous
    /// occurrence (`character-search-backward`; unbound — its readline
    /// default M-C-] isn't a decodable chord here, but `bind` can place
    /// it anywhere).
    CharacterSearchBackward,
    /// Ring the bell and do nothing else — readline's `abort` (C-g);
    /// pending states like an armed completion menu are already cleared
    /// by any non-completing key.
    Abort,
    /// Insert the next key literally (`quoted-insert`, C-v/C-q).
    QuotedInsert,
    /// Edit the line in `$VISUAL`/`$EDITOR` and execute the result
    /// (`edit-and-execute-command`, C-x C-e).
    EditAndExecuteCommand,
}

impl EditorAction {
    /// The action's readline command name (`kill-line`, `menu-complete`,
    /// …) — what a host's `bind` builtin prints and parses, so shells
    /// don't each maintain a drift-prone name table. Actions with no
    /// readline equivalent use the same kebab-case convention
    /// (`interrupt`, `delete-char-or-eof`).
    pub fn name(self) -> &'static str {
        // Exhaustive on purpose: a new variant fails to compile until it
        // gets a name here (and a row in `from_name`).
        match self {
            EditorAction::SelfInsert => "self-insert",
            EditorAction::AcceptLine => "accept-line",
            EditorAction::OperateAndGetNext => "operate-and-get-next",
            EditorAction::Interrupt => "interrupt",
            EditorAction::DeleteCharOrEof => "delete-char-or-eof",
            EditorAction::DeleteChar => "delete-char",
            EditorAction::BackwardDeleteChar => "backward-delete-char",
            EditorAction::BeginningOfLine => "beginning-of-line",
            EditorAction::EndOfLine => "end-of-line",
            EditorAction::ForwardChar => "forward-char",
            EditorAction::BackwardChar => "backward-char",
            EditorAction::ForwardWord => "forward-word",
            EditorAction::BackwardWord => "backward-word",
            EditorAction::KillLine => "kill-line",
            EditorAction::KillWholeLine => "kill-whole-line",
            EditorAction::DeleteHorizontalSpace => "delete-horizontal-space",
            EditorAction::UnixLineDiscard => "unix-line-discard",
            EditorAction::UnixWordRubout => "unix-word-rubout",
            EditorAction::KillWord => "kill-word",
            EditorAction::BackwardKillWord => "backward-kill-word",
            EditorAction::Yank => "yank",
            EditorAction::YankPop => "yank-pop",
            EditorAction::TransposeChars => "transpose-chars",
            EditorAction::TransposeWords => "transpose-words",
            EditorAction::UpcaseWord => "upcase-word",
            EditorAction::DowncaseWord => "downcase-word",
            EditorAction::CapitalizeWord => "capitalize-word",
            EditorAction::Undo => "undo",
            EditorAction::RevertLine => "revert-line",
            EditorAction::InsertLastArgument => "insert-last-argument",
            EditorAction::PreviousHistory => "previous-history",
            EditorAction::NextHistory => "next-history",
            EditorAction::BeginningOfHistory => "beginning-of-history",
            EditorAction::EndOfHistory => "end-of-history",
            EditorAction::HistorySearchBackward => "history-search-backward",
            EditorAction::HistorySearchForward => "history-search-forward",
            EditorAction::ReverseSearchHistory => "reverse-search-history",
            EditorAction::ForwardSearchHistory => "forward-search-history",
            EditorAction::ClearScreen => "clear-screen",
            EditorAction::Complete => "complete",
            EditorAction::MenuComplete => "menu-complete",
            EditorAction::MenuCompleteBackward => "menu-complete-backward",
            EditorAction::PossibleCompletions => "possible-completions",
            EditorAction::InsertCompletions => "insert-completions",
            EditorAction::CharacterSearch => "character-search",
            EditorAction::CharacterSearchBackward => "character-search-backward",
            EditorAction::Abort => "abort",
            EditorAction::QuotedInsert => "quoted-insert",
            EditorAction::EditAndExecuteCommand => "edit-and-execute-command",
        }
    }

    /// The inverse of [`name`](EditorAction::name): resolve a readline
    /// command name to the action, `None` for anything unrecognized
    /// (including names of readline commands not modeled here).
    pub fn from_name(name: &str) -> Option<EditorAction> {
        Some(match name {
            "self-insert" => EditorAction::SelfInsert,
            "accept-line" => EditorAction::AcceptLine,
            "operate-and-get-next" => EditorAction::OperateAndGetNext,
            "interrupt" => EditorAction::Interrupt,
            "delete-char-or-eof" => EditorAction::DeleteCharOrEof,
            "delete-char" => EditorAction::DeleteChar,
            "backward-delete-char" => EditorAction::BackwardDeleteChar,
            "beginning-of-line" => EditorAction::BeginningOfLine,
            "end-of-line" => EditorAction::EndOfLine,
            "forward-char" => EditorAction::ForwardChar,
            "backward-char" => EditorAction::BackwardChar,
            "forward-word" => EditorAction::ForwardWord,
            "backward-word" => EditorAction::BackwardWord,
            "kill-line" => EditorAction::KillLine,
            "kill-whole-line" => EditorAction::KillWholeLine,
            "delete-horizontal-space" => EditorAction::DeleteHorizontalSpace,
            "unix-line-discard" => EditorAction::UnixLineDiscard,
            "unix-word-rubout" => EditorAction::UnixWordRubout,
            "kill-word" => EditorAction::KillWord,
            "backward-kill-word" => EditorAction::BackwardKillWord,
            "yank" => EditorAction::Yank,
            "yank-pop" => EditorAction::YankPop,
            "transpose-chars" => EditorAction::TransposeChars,
            "transpose-words" => EditorAction::TransposeWords,
            "upcase-word" => EditorAction::UpcaseWord,
            "downcase-word" => EditorAction::DowncaseWord,
            "capitalize-word" => EditorAction::CapitalizeWord,
            "undo" => EditorAction::Undo,
            "revert-line" => EditorAction::RevertLine,
            "insert-last-argument" => EditorAction::InsertLastArgument,
            "previous-history" => EditorAction::PreviousHistory,
            "next-history" => EditorAction::NextHistory,
            "beginning-of-history" => EditorAction::BeginningOfHistory,
            "end-of-history" => EditorAction::EndOfHistory,
            "history-search-backward" => EditorAction::HistorySearchBackward,
            "history-search-forward" => EditorAction::HistorySearchForward,
            "reverse-search-history" => EditorAction::ReverseSearchHistory,
            "forward-search-history" => EditorAction::ForwardSearchHistory,
            "clear-screen" => EditorAction::ClearScreen,
            "complete" => EditorAction::Complete,
            "menu-complete" => EditorAction::MenuComplete,
            "menu-complete-backward" => EditorAction::MenuCompleteBackward,
            "possible-completions" => EditorAction::PossibleCompletions,
            "insert-completions" => EditorAction::InsertCompletions,
            "character-search" => EditorAction::CharacterSearch,
            "character-search-backward" => EditorAction::CharacterSearchBackward,
            "abort" => EditorAction::Abort,
            "quoted-insert" => EditorAction::QuotedInsert,
            "edit-and-execute-command" => EditorAction::EditAndExecuteCommand,
            _ => return None,
        })
    }
}

/// Formats as the readline command name ([`name`](EditorAction::name)),
/// so `bind -P`-style listings are just `format!("{action}")`.
impl std::fmt::Display for EditorAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// What ringing the terminal bell does — readline's `bell-style`
/// variable, set via [`Editor::set_bell_style`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BellStyle {
    /// No feedback.
    None,
    /// BEL — the terminal's audible beep (readline's default).
    #[default]
    Audible,
    /// A screen flash (reverse-video flip) instead of a beep.
    Visible,
}

/// A key's resolved binding: a named action, a host command tag
/// (`bind -x`), or explicitly nothing (`bind -r` masking a default).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Binding {
    Action(EditorAction),
    Host(String),
    Unbound,
}

/// The readline-variable knobs, copied into each `read_line`'s state.
#[derive(Debug, Clone, Copy)]
struct EditorConfig {
    completion_ignore_case: bool,
    show_all_if_ambiguous: bool,
    menu_complete: bool,
    bell: BellStyle,
    show_mode_in_prompt: bool,
    /// Case-insensitive history searching — incremental (C-r/C-s) and
    /// prefix (PageUp/PageDown) both (readline 8.1 `search-ignore-case`).
    search_ignore_case: bool,
    /// Candidate count at/past which the list prompts "Display all N
    /// possibilities?" first (readline `completion-query-items`; 0 = never).
    completion_query_items: usize,
    /// Appended after a unique, fully-inserted completion
    /// (readline's `rl_completion_append_character`; `None` = nothing).
    completion_append: Option<char>,
    /// Kill-ring depth (readline keeps 10 by default; 32 here, the
    /// crate's historical cap).
    max_kill_ring: usize,
    /// Undo-stack depth per line.
    max_undo: usize,
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig {
            completion_ignore_case: false,
            show_all_if_ambiguous: false,
            menu_complete: false,
            bell: BellStyle::default(),
            show_mode_in_prompt: false,
            search_ignore_case: false,
            completion_query_items: 100,
            completion_append: Some(' '),
            max_kill_ring: 32,
            max_undo: 200,
        }
    }
}

/// How a [`Editor::read_line`] call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadResult {
    /// A complete line (Enter).
    Line(String),
    /// Ctrl-C at the prompt.
    Interrupted,
    /// Ctrl-D on an empty line.
    Eof,
    /// The [`Editor::read_line_timeout`] deadline passed with no complete
    /// line (bash's `$TMOUT`: "timed out waiting for input").
    TimedOut,
}

/// Terminal size of stdout as `(columns, rows)`, or `None` when stdout is
/// not a terminal. What a shell needs to keep `$COLUMNS`/`$LINES` fresh
/// (bash 5's `checkwinsize` default).
pub fn terminal_size() -> Option<(u16, u16)> {
    term_sys::term_size_stdout()
}

thread_local! {
    /// Set by [`prepare_external_output`], consumed the next time the idle
    /// tick inside `read_line_raw`'s wait loop reaches its own repaint
    /// check (`lib.rs`'s self-heal branch) — at most one wait-loop pass
    /// away, since that loop runs on every "waiting for the next key"
    /// cycle regardless of whether anything actually changed.
    static EXTERNAL_OUTPUT_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Interrupt the current prompt/buffer's on-screen paint for output a
/// [`Hooks`] implementation wants to print outside the editor's own
/// rendering — e.g. a shell's background-job completion notice, printed
/// from [`Hooks::on_interrupted_read`] while idle at the prompt with a
/// partial line already typed. Moves to a fresh line immediately and marks
/// the region for a full repaint at the next safe point; the editor
/// redraws the prompt and buffer automatically once the hook call that
/// invoked this returns — the caller doesn't drive the repaint itself.
///
/// Call this *before* printing, not after: `render`'s own paint logic
/// assumes the cursor is exactly where the previous `render` call left it
/// (it moves up a fixed number of rows to get back to the top of the
/// region), so text written to the terminal without first resetting that
/// bookkeeping corrupts the next repaint's row math, not just its
/// content — the same reason the existing self-heal logic (an external
/// `stty` leaving the terminal cooked, or a mid-edit resize) already does
/// its own reset-then-newline *before* repainting, never after.
///
/// A no-op outside the raw-mode interactive editor (the piped-stdin path
/// has no on-screen paint to protect in the first place) — still safe to
/// call unconditionally.
pub fn prepare_external_output() -> io::Result<()> {
    EXTERNAL_OUTPUT_PENDING.with(|f| f.set(true));
    writeln!(io::stdout())
}

/// Run `f` with terminal echo off (a shell's `read -s`), restoring the
/// previous state on every exit path — including a panic in `f` — via an
/// internal RAII guard. When stdin is not a terminal there is no echo to
/// disable and `f` simply runs.
pub fn with_echo_disabled<T>(f: impl FnOnce() -> T) -> io::Result<T> {
    if !term_sys::isatty_stdin() {
        return Ok(f());
    }
    /// Restores the saved attributes on drop.
    struct EchoGuard(term_sys::Termios);
    impl Drop for EchoGuard {
        fn drop(&mut self) {
            let _ = term_sys::tcsetattr_stdin_drain(&self.0);
        }
    }
    let saved = term_sys::tcgetattr_stdin()?;
    let mut silent = saved;
    term_sys::clear_echo_flag(&mut silent);
    term_sys::tcsetattr_stdin_drain(&silent)?;
    let _guard = EchoGuard(saved);
    Ok(f())
}

/// The line editor: owns the history and the kill ring, both of which
/// persist across [`read_line`](Editor::read_line) calls within a session.
#[derive(Debug)]
pub struct Editor {
    history: Vec<String>,
    /// Per-entry epoch timestamps, parallel to `history` (`None` for
    /// entries loaded from a file without them).
    timestamps: Vec<Option<i64>>,
    /// The kill ring (readline's): survives across lines within a session.
    kill_ring: VecDeque<String>,
    /// Cap on history entries (readline's `stifle_history`); oldest are
    /// dropped past it. `usize::MAX` = unbounded, the default.
    max_history: usize,
    /// How many history entries are already in the history file, so
    /// `append_history` writes only the ones added since.
    persisted: usize,
    /// When set, a new entry erases earlier duplicates everywhere in the
    /// history, not just a consecutive repeat.
    dedup: bool,
    /// When set, `add_history_entry` skips lines starting with a space
    /// (bash `HISTCONTROL=ignorespace`).
    ignore_space: bool,
    /// C-o (`operate-and-get-next`): the history index the next
    /// `read_line` should pre-load, captured when the key accepted a
    /// recalled entry — together with the entry's text at capture time,
    /// so the entry can be re-located if `erasedups` (or any host
    /// history edit) shifts indices before the next call.
    next_recall: Option<(usize, String)>,
    /// When set, `save_history`/`append_history` write bash's `#<epoch>`
    /// timestamp comment before each entry (`HISTTIMEFORMAT`'s format).
    write_timestamps: bool,
    /// Host rebindings, overlaid on the default keymap (emacs and
    /// vi-insert modes; vi normal mode is not rebindable).
    bindings: Vec<(Key, Binding)>,
    /// The readline-variable knobs (`set_completion_ignore_case` …).
    cfg: EditorConfig,
}

/// The piped-stdin path: one line, no editing. A deadline, if set, is
/// honored between bytes via `poll`; a signal interrupting the read
/// fires [`Hooks::on_interrupted_read`] like the raw path does, so a
/// host running a piped script still gets its trap callback. When stdin
/// *is* a terminal (this path was taken because stdout is piped), the
/// prompt goes to stderr — bash's rule — so the interactive user still
/// sees where to type.
fn read_line_plain(
    prompt: &str,
    hooks: &dyn Hooks,
    deadline: Option<std::time::Instant>,
) -> io::Result<ReadResult> {
    if term_sys::isatty_stdin() && !prompt.is_empty() {
        let mut err = io::stderr().lock();
        err.write_all(prompt.as_bytes())?;
        err.flush()?;
    }
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        if let Some(d) = deadline {
            let Some(remaining) = d.checked_duration_since(std::time::Instant::now()) else {
                return Ok(ReadResult::TimedOut);
            };
            if !term_sys::poll_stdin(remaining.as_millis().min(i32::MAX as u128) as i32) {
                continue; // re-check the deadline, then poll again
            }
        }
        match io::stdin().read(&mut b) {
            Ok(0) => {
                if line.is_empty() {
                    return Ok(ReadResult::Eof);
                }
                break;
            }
            Ok(_) if b[0] == b'\n' => break,
            Ok(_) => line.push(b[0]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                hooks.on_interrupted_read();
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    // CRLF input: strip the carriage return too, like the non-Unix
    // buffered fallback does.
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(ReadResult::Line(
        String::from_utf8_lossy(&line).into_owned(),
    ))
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new()
    }
}

impl Editor {
    /// A fresh editor with empty history and kill ring.
    pub fn new() -> Self {
        Editor {
            history: Vec::new(),
            timestamps: Vec::new(),
            kill_ring: VecDeque::new(),
            max_history: usize::MAX,
            persisted: 0,
            dedup: false,
            ignore_space: false,
            next_recall: None,
            write_timestamps: false,
            bindings: Vec::new(),
            cfg: EditorConfig::default(),
        }
    }

    /// Rebind `keys` to `action`, replacing the default (or a previous
    /// rebinding) — readline's `bind '"\C-x": function'`. The key spec
    /// accepts readline's spellings: `\C-x`, `\M-f`, `\e[1;5C`, plus the
    /// usual backslash escapes. Bindings apply to the emacs and vi-insert
    /// keymaps; vi normal mode is fixed. Errors on a spec that does not
    /// parse to a single recognized key (multi-key chords are not
    /// supported).
    pub fn bind(&mut self, keys: &str, action: EditorAction) -> io::Result<()> {
        self.bind_internal(keys, Binding::Action(action))
    }

    /// Bind `keys` to a host command — bash's `bind -x`. When the key is
    /// pressed the editor suspends raw mode and calls
    /// [`Hooks::host_binding`] with `tag` and the current line/cursor;
    /// the host runs its command (with `READLINE_LINE`/`READLINE_POINT`
    /// semantics), and whatever it writes back becomes the buffer.
    pub fn bind_host(&mut self, keys: &str, tag: String) -> io::Result<()> {
        self.bind_internal(keys, Binding::Host(tag))
    }

    /// Remove any binding for `keys` — including the default, so the key
    /// does nothing (readline's `bind -r`). Rebind with
    /// [`bind`](Editor::bind) to restore behavior.
    pub fn unbind(&mut self, keys: &str) -> io::Result<()> {
        self.bind_internal(keys, Binding::Unbound)
    }

    fn bind_internal(&mut self, keys: &str, binding: Binding) -> io::Result<()> {
        let key = parse_key_spec(keys).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unparseable key spec: {keys:?}"),
            )
        })?;
        if let Some(slot) = self.bindings.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = binding;
        } else {
            self.bindings.push((key, binding));
        }
        Ok(())
    }

    /// The current action bindings — defaults with rebindings applied —
    /// as (key spec, action) pairs (a shell's `bind -P`). Host-command
    /// bindings and self-inserting character keys are not listed.
    pub fn bindings(&self) -> impl Iterator<Item = (String, EditorAction)> + '_ {
        let defaults = DEFAULT_BINDINGS
            .iter()
            .filter(|(k, _)| !self.bindings.iter().any(|(ck, _)| ck == k))
            .map(|(k, a)| (key_spec(k), *a));
        let custom = self.bindings.iter().filter_map(|(k, b)| match b {
            Binding::Action(a) => Some((key_spec(k), *a)),
            _ => None,
        });
        defaults.chain(custom)
    }

    /// The host-command bindings, as (key spec, tag) pairs — the
    /// [`bind_host`](Editor::bind_host) entries that
    /// [`bindings`](Editor::bindings) deliberately omits, so a shell can
    /// list them (bash's `bind -X`).
    pub fn host_bindings(&self) -> impl Iterator<Item = (String, &str)> + '_ {
        self.bindings.iter().filter_map(|(k, b)| match b {
            Binding::Host(tag) => Some((key_spec(k), tag.as_str())),
            _ => None,
        })
    }

    /// Case-insensitive completion matching (readline's
    /// `completion-ignore-case`): the longest-common-prefix step compares
    /// candidates ignoring case (the first candidate's case is inserted).
    /// Off by default, like readline.
    pub fn set_completion_ignore_case(&mut self, on: bool) {
        self.cfg.completion_ignore_case = on;
    }

    /// List completion candidates immediately when a completion is
    /// ambiguous, instead of on the second Tab (readline's
    /// `show-all-if-ambiguous`). Off by default, like readline.
    pub fn set_show_all_if_ambiguous(&mut self, on: bool) {
        self.cfg.show_all_if_ambiguous = on;
    }

    /// Make Tab cycle through the candidates directly, inserting the
    /// first match immediately (readline's `menu-complete` bound in place
    /// of `complete`). Off by default, like readline.
    pub fn set_menu_complete(&mut self, on: bool) {
        self.cfg.menu_complete = on;
    }

    /// What ringing the bell does (readline's `bell-style`): audible
    /// (the default, like readline), visible, or nothing. The editor
    /// rings on completion with no candidates.
    pub fn set_bell_style(&mut self, style: BellStyle) {
        self.cfg.bell = style;
    }

    /// When enabled, adding an entry removes earlier duplicates anywhere
    /// in the history (bash `HISTCONTROL=erasedups`; fish's behavior),
    /// not just a consecutive repeat. Off by default.
    pub fn set_history_dedup(&mut self, on: bool) {
        self.dedup = on;
    }

    /// When enabled, [`add_history_entry`](Editor::add_history_entry)
    /// skips lines that begin with a space character (bash
    /// `HISTCONTROL=ignorespace`; zsh `HIST_IGNORE_SPACE`). Off by
    /// default. Entries loaded from a file are kept regardless.
    pub fn set_history_ignore_space(&mut self, on: bool) {
        self.ignore_space = on;
    }

    /// Show the keymap mode in front of the prompt (readline's
    /// `show-mode-in-prompt`): `(ins)`/`(cmd)` in vi mode, `@` in emacs
    /// mode — readline's default mode strings. Off by default.
    pub fn set_show_mode_in_prompt(&mut self, on: bool) {
        self.cfg.show_mode_in_prompt = on;
    }

    /// Case-insensitive history searching (readline 8.1's
    /// `search-ignore-case`): C-r/C-s incremental search and the
    /// PageUp/PageDown prefix search both match ignoring case. Off by
    /// default, like readline.
    pub fn set_search_ignore_case(&mut self, on: bool) {
        self.cfg.search_ignore_case = on;
    }

    /// The character appended after a unique, fully-inserted completion —
    /// readline's `rl_completion_append_character`, so accepting the only
    /// match leaves the cursor ready for the next word. The default is a
    /// space, like readline; `None` appends nothing.
    pub fn set_completion_append_character(&mut self, c: Option<char>) {
        self.cfg.completion_append = c;
    }

    /// Ask "Display all N possibilities? (y or n)" before printing a
    /// candidate list of `n` or more entries (readline's
    /// `completion-query-items`). The default is 100, like readline;
    /// 0 disables the question so every list prints immediately.
    pub fn set_completion_query_items(&mut self, n: usize) {
        self.cfg.completion_query_items = n;
    }

    /// Cap the kill ring at `n` entries; the oldest rotate out as new
    /// kills arrive. Applies immediately. The default is 32.
    pub fn set_max_kill_ring_len(&mut self, n: usize) {
        self.cfg.max_kill_ring = n;
        while self.kill_ring.len() > n {
            self.kill_ring.pop_front();
        }
    }

    /// Cap the per-line undo stack at `n` snapshots; the oldest drop
    /// past it. The default is 200.
    pub fn set_max_undo_len(&mut self, n: usize) {
        self.cfg.max_undo = n;
    }

    /// Cap the history at `n` entries (readline's `stifle_history`,
    /// bash's `HISTSIZE`): the oldest entries are dropped as new ones
    /// arrive. Applies immediately and to future `add_history_entry`
    /// calls. The default is unbounded.
    pub fn set_max_history_len(&mut self, n: usize) {
        self.max_history = n;
        self.trim_history();
    }

    fn trim_history(&mut self) {
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(..excess);
            self.timestamps.drain(..excess);
            self.persisted = self.persisted.saturating_sub(excess);
        }
    }

    /// The history entries, oldest first.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Per-entry epoch timestamps, parallel to [`history`](Editor::history)
    /// — `None` for entries loaded from a file without them. What a shell's
    /// `history` builtin renders through `strftime($HISTTIMEFORMAT)`.
    pub fn history_timestamps(&self) -> &[Option<i64>] {
        &self.timestamps
    }

    /// When enabled, `save_history` and `append_history` precede each entry
    /// with bash's `#<epoch>` timestamp comment — the `HISTTIMEFORMAT` file
    /// format. Off by default, so existing plain history files are not
    /// rewritten into the timestamped format behind the user's back
    /// (bash likewise writes timestamps only when `HISTTIMEFORMAT` is set).
    /// `load_history` understands both formats regardless of this toggle.
    pub fn set_history_timestamps(&mut self, on: bool) {
        self.write_timestamps = on;
    }

    /// Append to history, skipping a consecutive duplicate. A multi-line
    /// entry (a bracketed paste) is joined with `; ` — bash's `cmdhist`
    /// behavior — so recall and the line-oriented history file both work.
    /// Lines starting with a space are skipped under
    /// [`set_history_ignore_space`](Editor::set_history_ignore_space).
    /// The entry is stamped with the current time (rendered to the file
    /// only under [`set_history_timestamps`](Editor::set_history_timestamps)).
    pub fn add_history_entry(&mut self, line: &str) {
        if self.ignore_space && line.starts_with(' ') {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs() as i64);
        self.add_entry(line, now);
    }

    fn add_entry(&mut self, line: &str, timestamp: Option<i64>) {
        let entry = if line.contains('\n') {
            line.replace('\n', "; ")
        } else {
            line.to_string()
        };
        if self.dedup {
            let mut i = 0;
            while i < self.history.len() {
                if self.history[i] == entry {
                    self.history.remove(i);
                    self.timestamps.remove(i);
                    // A removed entry below the persisted watermark shifts
                    // it down; the file keeps the stale copy until the next
                    // full save_history.
                    if i < self.persisted {
                        self.persisted -= 1;
                    }
                } else {
                    i += 1;
                }
            }
        }
        if self.history.last() != Some(&entry) {
            self.history.push(entry);
            self.timestamps.push(timestamp);
            self.trim_history();
        }
    }

    /// Replace the whole history in place — a shell resynchronizing after
    /// its `history -c` / `history -d` builtin — without rebuilding the
    /// editor (the kill ring and other session state survive). Entries are
    /// treated as already persisted: a following `append_history` writes
    /// only entries added *after* this call, mirroring bash, where deletion
    /// edits the in-memory list and the file catches up on the next full
    /// write. Timestamps are cleared (the caller's mirror is line-only).
    pub fn replace_history(&mut self, entries: Vec<String>) {
        self.history = entries
            .into_iter()
            .map(|e| {
                if e.contains('\n') {
                    e.replace('\n', "; ")
                } else {
                    e
                }
            })
            .collect();
        self.timestamps = vec![None; self.history.len()];
        self.persisted = self.history.len();
        self.trim_history();
    }

    /// Load history from `path`. Plain lines; a `#<epoch>` comment line
    /// (bash's `HISTTIMEFORMAT` file format) stamps the entry that follows
    /// it; a leading `#V2` header (the format `rustyline`'s `FileHistory`
    /// writes, for hosts migrating) is skipped. Files with and without
    /// timestamps both round-trip.
    ///
    /// A history *entry* can itself look like a timestamp comment —
    /// interactive shells store comment lines, so a user who typed `#42`
    /// has exactly that in the file. The loader keeps the recoverable
    /// cases as entries: only epoch-scale stamps (nine or more digits)
    /// count as timestamps, and a stamp with nothing after it is an
    /// entry. A lone comment line that *is* an epoch-scale number
    /// followed by another entry remains inherently ambiguous in bash's
    /// format and is read as a timestamp.
    pub fn load_history(&mut self, path: &std::path::Path) -> io::Result<()> {
        fn stamp_of(line: &str) -> Option<i64> {
            let digits = line.strip_prefix('#')?;
            if digits.len() >= 9 && digits.bytes().all(|b| b.is_ascii_digit()) {
                digits.parse().ok()
            } else {
                None
            }
        }
        let text = std::fs::read_to_string(path)?;
        let mut parsed: Vec<(&str, Option<i64>)> = Vec::new();
        let mut lines = text.lines().enumerate().peekable();
        while let Some((i, line)) = lines.next() {
            if i == 0 && line == "#V2" {
                continue;
            }
            if let Some(ts) = stamp_of(line) {
                match lines.peek() {
                    // The stamp applies to the next line — even one that
                    // is itself stamp-shaped: the writer always pairs
                    // stamp+entry, so an adjacent stamp-shaped line is a
                    // `#<digits>` command being recalled.
                    Some(&(_, next)) if !next.is_empty() => {
                        let (_, entry) = lines.next().expect("peeked");
                        parsed.push((entry, Some(ts)));
                    }
                    // Dangling at EOF (or before a blank): a stamp is
                    // never written without its entry, so this was a
                    // real `#<digits>` entry.
                    _ => parsed.push((line, None)),
                }
                continue;
            }
            if !line.is_empty() {
                parsed.push((line, None));
            }
        }
        if self.dedup {
            // erasedups over the whole load in one backward pass (the
            // last occurrence wins — exactly what sequential adds
            // produce), instead of `add_entry`'s linear scan per line,
            // which made loading a large file O(n²).
            let mut combined: Vec<(String, Option<i64>)> = std::mem::take(&mut self.history)
                .into_iter()
                .zip(std::mem::take(&mut self.timestamps))
                .collect();
            combined.extend(parsed.into_iter().map(|(e, ts)| (e.to_string(), ts)));
            let mut keep: Vec<bool> = vec![false; combined.len()];
            {
                let mut seen = std::collections::HashSet::new();
                for (i, (entry, _)) in combined.iter().enumerate().rev() {
                    keep[i] = seen.insert(entry.as_str());
                }
            }
            for (kept, (entry, ts)) in keep.into_iter().zip(combined) {
                if kept {
                    self.history.push(entry);
                    self.timestamps.push(ts);
                }
            }
            self.trim_history();
        } else {
            for (entry, ts) in parsed {
                self.add_entry(entry, ts);
            }
        }
        self.persisted = self.history.len();
        Ok(())
    }

    /// One entry (or a `persisted..` tail) in the on-disk format:
    /// `#<epoch>` comment lines only under `set_history_timestamps`.
    fn format_entries(&self, from: usize) -> String {
        let mut out = String::new();
        for (entry, ts) in self.history[from..].iter().zip(&self.timestamps[from..]) {
            if self.write_timestamps
                && let Some(ts) = ts
            {
                out.push_str(&format!("#{ts}\n"));
            }
            out.push_str(entry);
            out.push('\n');
        }
        out
    }

    /// Write the history to `path`, one entry per line (preceded by a
    /// `#<epoch>` timestamp line when
    /// [`set_history_timestamps`](Editor::set_history_timestamps) is on).
    /// The write is atomic (temp file + rename, so a crash mid-write
    /// can't truncate an existing history) and a new file is created
    /// mode 0600 on Unix, like bash's history file — history routinely
    /// contains secrets. An existing file keeps its permissions.
    pub fn save_history(&mut self, path: &std::path::Path) -> io::Result<()> {
        write_history_atomic(path, self.format_entries(0).as_bytes())?;
        self.persisted = self.history.len();
        Ok(())
    }

    /// Append only the entries added since the last `load_history`,
    /// `save_history`, or `append_history` call — bash's `histappend`:
    /// concurrent sessions interleave instead of overwriting each other.
    /// A file this call creates is mode 0600 on Unix, like bash's.
    pub fn append_history(&mut self, path: &std::path::Path) -> io::Result<()> {
        let from = self.persisted.min(self.history.len());
        if from < self.history.len() {
            use std::io::Write as _;
            let mut opts = std::fs::OpenOptions::new();
            opts.create(true).append(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            let mut f = opts.open(path)?;
            f.write_all(self.format_entries(from).as_bytes())?;
        }
        self.persisted = self.history.len();
        Ok(())
    }

    /// Read one line interactively. `rprompt` is the already-expanded
    /// right-side prompt text (zsh's `$RPS1`), or empty for none.
    pub fn read_line(
        &mut self,
        prompt: &str,
        rprompt: &str,
        hooks: &dyn Hooks,
    ) -> io::Result<ReadResult> {
        self.read_line_timeout(prompt, rprompt, hooks, None)
    }

    /// [`read_line`](Editor::read_line) with a deadline: when no complete
    /// line has been entered within `timeout` (measured from the call, not
    /// from the last keystroke — readline's `rl_readline_state` timeout,
    /// bash's `$TMOUT`), returns [`ReadResult::TimedOut`]. `None` never
    /// times out. On non-Unix builds the timeout is ignored (the fallback
    /// is a blocking buffered read).
    pub fn read_line_timeout(
        &mut self,
        prompt: &str,
        rprompt: &str,
        hooks: &dyn Hooks,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<ReadResult> {
        self.read_line_inner(prompt, rprompt, hooks, timeout, None)
    }

    /// [`read_line`](Editor::read_line) with the buffer pre-seeded:
    /// `initial` is `(left, right)` text placed in the buffer with the
    /// cursor between the two — rustyline's `readline_with_initial`,
    /// zsh's `print -z`. How a host implements `fc`-style edit-and-rerun
    /// or offers a correction the user can accept or edit. On a non-tty
    /// stdin (and on non-Unix builds) there is no buffer to seed and the
    /// text is ignored.
    pub fn read_line_with_initial(
        &mut self,
        prompt: &str,
        rprompt: &str,
        hooks: &dyn Hooks,
        initial: (&str, &str),
    ) -> io::Result<ReadResult> {
        self.read_line_inner(prompt, rprompt, hooks, None, Some(initial))
    }

    /// The full-form read: both a deadline and a pre-seeded buffer —
    /// [`read_line_timeout`](Editor::read_line_timeout) and
    /// [`read_line_with_initial`](Editor::read_line_with_initial)
    /// combined, so a `$TMOUT` host doesn't have to give up seeding.
    pub fn read_line_with_initial_timeout(
        &mut self,
        prompt: &str,
        rprompt: &str,
        hooks: &dyn Hooks,
        initial: (&str, &str),
        timeout: Option<std::time::Duration>,
    ) -> io::Result<ReadResult> {
        self.read_line_inner(prompt, rprompt, hooks, timeout, Some(initial))
    }

    fn read_line_inner(
        &mut self,
        prompt: &str,
        rprompt: &str,
        hooks: &dyn Hooks,
        timeout: Option<std::time::Duration>,
        initial: Option<(&str, &str)>,
    ) -> io::Result<ReadResult> {
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        // A non-tty stdin (a script piped into an "interactive"
        // host) can't enter raw mode, and a non-tty stdout (the host
        // piped into `tee`) has nothing to paint on — repaint escape
        // sequences would just be sprayed into the pipe. Either way,
        // fall back to a plain silent read, like readline does.
        if !term_sys::isatty_stdin() || !term_sys::isatty_stdout() {
            return read_line_plain(prompt, hooks, deadline);
        }
        read_line_raw(self, prompt, rprompt, hooks, deadline, initial)
    }
}

/// Replace `path` atomically: write a sibling temp file, then rename it
/// over the target — a crash mid-write leaves the old file intact
/// instead of a truncated one. A new file ends up mode 0600 on Unix
/// (bash creates its history file 0600 too — history carries secrets);
/// an existing file's permissions are preserved across the rename.
fn write_history_atomic(path: &std::path::Path, data: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp.{}", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        // Keep a pre-existing history file's permissions; only a fresh
        // file gets the restrictive default.
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = f.set_permissions(meta.permissions());
        }
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        // The rename itself lives in the directory entry: sync the
        // parent too, or a crash right here can resurrect the old file.
        // (Best-effort — opening a directory read-only fails on some
        // platforms, and the data is already safe in the temp file.)
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
        if let Ok(d) = std::fs::File::open(dir.unwrap_or(std::path::Path::new("."))) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// One decoded input event.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Char(char),
    Ctrl(char), // Ctrl('a') for ^A …; Ctrl('_') for ^_
    Alt(char),
    AltBackspace, // ESC DEL — backward-kill-word
    Enter,
    Tab,
    /// Shift-Tab (`CSI Z`, back-tab) — reverse menu cycling.
    BackTab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    WordLeft,  // Ctrl/Alt-Left
    WordRight, // Ctrl/Alt-Right
    PageUp,
    PageDown,
    /// A bracketed paste: the whole pasted text as one event, so tabs and
    /// escape bytes inside it insert literally instead of firing bindings.
    Paste(String),
    Esc,
    Other,
}

/// Raw-mode RAII guard: restores the saved termios on drop, whatever the
/// exit path. Keeps both states so `suspend`/`resume` can hand the
/// terminal to an external `$EDITOR` (C-x C-e) and take it back.
struct RawMode {
    saved: term_sys::Termios,
    raw: term_sys::Termios,
}

impl RawMode {
    fn enable() -> io::Result<RawMode> {
        let saved = term_sys::tcgetattr_stdin()?;
        // Input: no Ctrl-S/Q flow control (freeing C-s for forward
        // search), no CR→NL mangling. Local: no canonical buffering,
        // no echo, no signal generation (^C becomes a key we handle),
        // no ^V. Output stays cooked so ordinary `println!` keeps
        // working for lists and job notices.
        let mut raw = saved;
        term_sys::apply_raw_flags(&mut raw);
        term_sys::tcsetattr_stdin_drain(&raw)?;
        Ok(RawMode { saved, raw })
    }

    /// Back to the shell's normal (cooked) state, for an external editor.
    fn suspend(&self) {
        let _ = term_sys::tcsetattr_stdin_drain(&self.saved);
    }

    fn resume(&self) {
        let _ = term_sys::tcsetattr_stdin_drain(&self.raw);
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = term_sys::tcsetattr_stdin_drain(&self.saved);
    }
}

/// Bracketed-paste RAII guard: terminals wrap a paste in
/// `ESC[200~ … ESC[201~` while enabled, and the decoder turns that into a
/// single `Key::Paste` event.
struct BracketedPaste;

impl BracketedPaste {
    fn enable() -> BracketedPaste {
        // A guard constructor can't propagate a write error; a dead
        // terminal will surface from the next fallible paint instead.
        let _ = write!(io::stdout(), "\x1b[?2004h");
        let _ = io::stdout().flush();
        BracketedPaste
    }
}

impl Drop for BracketedPaste {
    fn drop(&mut self) {
        let _ = write!(io::stdout(), "\x1b[?2004l");
        let _ = io::stdout().flush();
    }
}

thread_local! {
    /// Bytes already pulled from fd 0 by one `read_stdin_chunk` call but
    /// not yet handed to the decoder. `read_byte` refills this with a
    /// single syscall reading whatever the kernel currently has queued,
    /// instead of one syscall per byte — the difference between ~1 and
    /// ~20,000 syscalls for a 20 KB paste. Thread-local (not
    /// per-`Editor`) because it holds real undelivered bytes from the
    /// fd, which must survive exactly as long as the fd's read position
    /// does — including across separate `read_line` calls, when a burst
    /// of typing outruns processing.
    static STDIN_BUF: std::cell::RefCell<(Vec<u8>, usize)> =
        const { std::cell::RefCell::new((Vec::new(), 0)) };
}

/// Whether fd 0 has a byte ready within `ms` milliseconds — the lone-ESC
/// vs escape-sequence disambiguation. Bytes already sitting in
/// [`STDIN_BUF`] count as ready with no syscall; this is what must agree
/// with `read_byte`'s buffer, or the lone-ESC poll would lie exactly the
/// way `io::Stdin`'s hidden buffer used to (see `read_byte`).
fn input_ready(ms: i32) -> bool {
    let buffered = STDIN_BUF.with(|b| {
        let (data, pos) = &*b.borrow();
        *pos < data.len()
    });
    buffered || term_sys::poll_stdin(ms)
}

/// One byte off fd 0, served from [`STDIN_BUF`] when it has one, else
/// refilled by a single `read` syscall — deliberately *not* through
/// `io::stdin()`, whose *own* userspace buffer would swallow the rest of
/// an escape sequence and make `input_ready`'s `poll` lie about it (the
/// arrow keys literally didn't work through the buffered reader). The
/// difference here is that this buffer *is* what `input_ready` consults,
/// so the two can never disagree the way `io::Stdin`'s opaque one did.
fn read_byte(hooks: &dyn Hooks) -> io::Result<Option<u8>> {
    if let Some(b) = STDIN_BUF.with(|b| {
        let (data, pos) = &mut *b.borrow_mut();
        (*pos < data.len()).then(|| {
            let b = data[*pos];
            *pos += 1;
            b
        })
    }) {
        return Ok(Some(b));
    }
    loop {
        let mut chunk = [0u8; 4096];
        match term_sys::read_stdin_chunk(&mut chunk) {
            Ok(0) => return Ok(None), // EOF
            Ok(n) => {
                return Ok(STDIN_BUF.with(|b| {
                    let (data, pos) = &mut *b.borrow_mut();
                    data.clear();
                    data.extend_from_slice(&chunk[..n]);
                    *pos = 1;
                    Some(chunk[0])
                }));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                // A signal (e.g. a deferred TERM) landed mid-read; let
                // trap machinery see it at the next safe point and
                // keep reading.
                hooks.on_interrupted_read();
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

/// [`read_byte`], but giving up (`Ok(None)`) when no byte arrives within
/// `ms` milliseconds — bounds the reads inside multi-byte sequences so a
/// half-delivered escape sequence or UTF-8 character can't block the
/// editor (or hold a `read_line_timeout` past its deadline).
fn read_byte_timeout(hooks: &dyn Hooks, ms: i32) -> io::Result<Option<u8>> {
    if !input_ready(ms) {
        return Ok(None);
    }
    read_byte(hooks)
}

/// Assemble one UTF-8 character whose first byte is `first`. A stray
/// continuation byte (or an invalid lead) becomes U+FFFD immediately
/// instead of swallowing the following keystrokes as continuations; the
/// continuation reads are also bounded, so a truncated sequence cannot
/// block the read.
fn read_utf8(hooks: &dyn Hooks, first: u8) -> io::Result<char> {
    let need = match first {
        0x00..=0x7f => 0,
        0xc0..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf4 => 3,
        // 0x80–0xbf (a continuation with no lead) and 0xf5–0xff (never
        // valid UTF-8): not the start of a character.
        _ => return Ok('\u{fffd}'),
    };
    let mut buf = vec![first];
    for _ in 0..need {
        if let Some(b) = read_byte_timeout(hooks, 200)? {
            buf.push(b);
        } else {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf)
        .chars()
        .next()
        .unwrap_or('\u{fffd}'))
}

/// Map a CSI escape sequence's final byte (plus parameters) to a key —
/// pure, so the quirk table is unit-testable. Modifier parameters `1;5`
/// (Ctrl) and `1;3` (Alt) on the arrows become word motions.
fn csi_key(params: &str, final_byte: u8) -> Key {
    match (params, final_byte) {
        ("1;5" | "1;3", b'C') => Key::WordRight,
        ("1;5" | "1;3", b'D') => Key::WordLeft,
        (_, b'A') => Key::Up,
        (_, b'B') => Key::Down,
        (_, b'C') => Key::Right,
        (_, b'D') => Key::Left,
        (_, b'H') => Key::Home,
        (_, b'F') => Key::End,
        ("1", b'~') | ("7", b'~') => Key::Home,
        ("4", b'~') | ("8", b'~') => Key::End,
        ("3", b'~') => Key::Delete,
        ("5", b'~') => Key::PageUp,
        ("6", b'~') => Key::PageDown,
        // Back-tab (Shift-Tab); some terminals send it with modifier
        // parameters (`1;2Z`), hence the wildcard.
        (_, b'Z') => Key::BackTab,
        _ => Key::Other,
    }
}

/// The default emacs / vi-insert keymap, as data: what dispatch consults
/// for an unrebound key, and what [`Editor::bindings`] lists. Character
/// keys (self-insert) and the hardcoded C-x chord prefix are not rows.
static DEFAULT_BINDINGS: &[(Key, EditorAction)] = &[
    (Key::Enter, EditorAction::AcceptLine),
    (Key::Ctrl('c'), EditorAction::Interrupt),
    (Key::Ctrl('d'), EditorAction::DeleteCharOrEof),
    (Key::Delete, EditorAction::DeleteChar),
    // C-h arrives as the 0x08 Backspace byte, so it is the same row.
    (Key::Backspace, EditorAction::BackwardDeleteChar),
    (Key::Home, EditorAction::BeginningOfLine),
    (Key::Ctrl('a'), EditorAction::BeginningOfLine),
    (Key::End, EditorAction::EndOfLine),
    (Key::Ctrl('e'), EditorAction::EndOfLine),
    (Key::Right, EditorAction::ForwardChar),
    (Key::Ctrl('f'), EditorAction::ForwardChar),
    (Key::Left, EditorAction::BackwardChar),
    (Key::Ctrl('b'), EditorAction::BackwardChar),
    (Key::WordRight, EditorAction::ForwardWord),
    (Key::Alt('f'), EditorAction::ForwardWord),
    (Key::WordLeft, EditorAction::BackwardWord),
    (Key::Alt('b'), EditorAction::BackwardWord),
    (Key::Ctrl('k'), EditorAction::KillLine),
    (Key::Alt('\\'), EditorAction::DeleteHorizontalSpace),
    (Key::Ctrl('u'), EditorAction::UnixLineDiscard),
    (Key::Ctrl('w'), EditorAction::UnixWordRubout),
    (Key::Alt('d'), EditorAction::KillWord),
    (Key::AltBackspace, EditorAction::BackwardKillWord),
    (Key::Ctrl('y'), EditorAction::Yank),
    (Key::Alt('y'), EditorAction::YankPop),
    (Key::Ctrl('t'), EditorAction::TransposeChars),
    (Key::Alt('t'), EditorAction::TransposeWords),
    (Key::Alt('u'), EditorAction::UpcaseWord),
    (Key::Alt('l'), EditorAction::DowncaseWord),
    (Key::Alt('c'), EditorAction::CapitalizeWord),
    (Key::Ctrl('_'), EditorAction::Undo),
    (Key::Ctrl('z'), EditorAction::Undo),
    (Key::Alt('r'), EditorAction::RevertLine),
    (Key::Alt('.'), EditorAction::InsertLastArgument),
    (Key::Alt('_'), EditorAction::InsertLastArgument),
    (Key::Up, EditorAction::PreviousHistory),
    (Key::Ctrl('p'), EditorAction::PreviousHistory),
    (Key::Down, EditorAction::NextHistory),
    (Key::Ctrl('n'), EditorAction::NextHistory),
    (Key::Alt('<'), EditorAction::BeginningOfHistory),
    (Key::Alt('>'), EditorAction::EndOfHistory),
    (Key::PageUp, EditorAction::HistorySearchBackward),
    (Key::Alt('p'), EditorAction::HistorySearchBackward),
    (Key::PageDown, EditorAction::HistorySearchForward),
    (Key::Alt('n'), EditorAction::HistorySearchForward),
    (Key::Ctrl('r'), EditorAction::ReverseSearchHistory),
    (Key::Ctrl('s'), EditorAction::ForwardSearchHistory),
    (Key::Ctrl('l'), EditorAction::ClearScreen),
    (Key::Ctrl('o'), EditorAction::OperateAndGetNext),
    (Key::Tab, EditorAction::Complete),
    (Key::BackTab, EditorAction::MenuCompleteBackward),
    (Key::Alt('?'), EditorAction::PossibleCompletions),
    (Key::Alt('*'), EditorAction::InsertCompletions),
    (Key::Ctrl(']'), EditorAction::CharacterSearch),
    (Key::Ctrl('g'), EditorAction::Abort),
    (Key::Ctrl('v'), EditorAction::QuotedInsert),
    (Key::Ctrl('q'), EditorAction::QuotedInsert),
];

/// The default action for a key: character keys self-insert, everything
/// else comes from the [`DEFAULT_BINDINGS`] table.
fn default_action(key: &Key) -> Option<EditorAction> {
    if matches!(key, Key::Char(_)) {
        return Some(EditorAction::SelfInsert);
    }
    DEFAULT_BINDINGS
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, a)| *a)
}

/// Parse a readline key spec — `\C-x`, `\M-f`, `\e[1;5C`, backslash
/// escapes (`\e \t \r \n \a \d \\ \xHH \NNN`), plain characters — into
/// the single key it decodes to. `None` for anything unparseable,
/// including multi-key chords.
fn parse_key_spec(spec: &str) -> Option<Key> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next()? {
            'C' => {
                if chars.next()? != '-' {
                    return None;
                }
                // The control target may itself be escaped (`\C-\\`).
                let t = match chars.next()? {
                    '\\' => chars.next()?,
                    t => t,
                };
                bytes.push(match t {
                    '?' => 0x7f,
                    t if t.is_ascii() => (t as u8) & 0x1f,
                    _ => return None,
                });
            }
            // Meta is an ESC prefix; the rest of the spec parses as
            // usual, so `\M-\C-?` works.
            'M' => {
                if chars.next()? != '-' {
                    return None;
                }
                bytes.push(0x1b);
            }
            'e' | 'E' => bytes.push(0x1b),
            '\\' => bytes.push(b'\\'),
            'a' => bytes.push(0x07),
            'b' => bytes.push(0x08),
            'd' => bytes.push(0x7f),
            'f' => bytes.push(0x0c),
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            't' => bytes.push(b'\t'),
            'v' => bytes.push(0x0b),
            'x' => {
                let mut v: u32 = 0;
                let mut seen = 0;
                while seen < 2
                    && let Some(d) = chars.peek().and_then(|c| c.to_digit(16))
                {
                    v = v * 16 + d;
                    chars.next();
                    seen += 1;
                }
                if seen == 0 {
                    return None;
                }
                bytes.push(v as u8);
            }
            d @ '0'..='7' => {
                let mut v: u32 = d.to_digit(8).unwrap();
                let mut seen = 1;
                while seen < 3
                    && let Some(d) = chars.peek().and_then(|c| c.to_digit(8))
                {
                    v = v * 8 + d;
                    chars.next();
                    seen += 1;
                }
                if v > 0xff {
                    return None;
                }
                bytes.push(v as u8);
            }
            _ => return None,
        }
    }
    decode_key_bytes(&bytes)
}

/// Decode a complete byte sequence into exactly one key — the pure twin
/// of [`read_key`]'s streaming decoder, for the binding parser. `None`
/// when the bytes are empty, leave a remainder, or decode to nothing
/// recognizable.
fn decode_key_bytes(bytes: &[u8]) -> Option<Key> {
    let (&b, rest) = bytes.split_first()?;
    let one = |key: Key| if rest.is_empty() { Some(key) } else { None };
    match b {
        b'\r' | b'\n' => one(Key::Enter),
        b'\t' => one(Key::Tab),
        0x7f | 0x08 => one(Key::Backspace),
        0x1b => decode_escape_bytes(rest),
        // The C0 bytes outside ^A..^Z, readline-spelled: ^@ ^\ ^] ^^ ^_.
        0x00 => one(Key::Ctrl('@')),
        0x1c => one(Key::Ctrl('\\')),
        0x1d => one(Key::Ctrl(']')),
        0x1e => one(Key::Ctrl('^')),
        0x1f => one(Key::Ctrl('_')),
        0x01..=0x1a => one(Key::Ctrl((b - 1 + b'a') as char)),
        _ => {
            let s = std::str::from_utf8(bytes).ok()?;
            let mut chars = s.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Some(Key::Char(c))
        }
    }
}

/// The post-ESC half of [`decode_key_bytes`].
fn decode_escape_bytes(rest: &[u8]) -> Option<Key> {
    let Some((&b, rest)) = rest.split_first() else {
        return Some(Key::Esc);
    };
    let one = |key: Key| if rest.is_empty() { Some(key) } else { None };
    match b {
        b'[' => {
            let params_len = rest
                .iter()
                .take_while(|&&c| c.is_ascii_digit() || c == b';')
                .count();
            let (params, fin) = rest.split_at(params_len);
            let (&final_byte, tail) = fin.split_first()?;
            if !tail.is_empty() {
                return None;
            }
            match csi_key(std::str::from_utf8(params).ok()?, final_byte) {
                Key::Other => None,
                key => Some(key),
            }
        }
        b'O' => {
            let (&f, tail) = rest.split_first()?;
            if !tail.is_empty() {
                return None;
            }
            match f {
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                _ => None,
            }
        }
        0x7f => one(Key::AltBackspace),
        c if c.is_ascii_graphic() => one(Key::Alt(c as char)),
        // Meta + a multibyte character (`\M-ö`): the ESC prefix followed
        // by one whole UTF-8 sequence.
        c if c >= 0x80 => {
            let mut bytes = vec![c];
            bytes.extend_from_slice(rest);
            let mut chars = std::str::from_utf8(&bytes).ok()?.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Some(Key::Alt(ch))
        }
        _ => None,
    }
}

/// Render a key back to a readline-style spec — [`Editor::bindings`]'s
/// output, chosen so it round-trips through [`parse_key_spec`].
fn key_spec(key: &Key) -> String {
    // A literal backslash must be spelled `\\` or the spec won't parse.
    fn esc(c: char) -> String {
        if c == '\\' {
            "\\\\".to_string()
        } else {
            c.to_string()
        }
    }
    match key {
        Key::Char(c) => esc(*c),
        Key::Ctrl(c) => format!("\\C-{}", esc(*c)),
        Key::Alt(c) => format!("\\M-{}", esc(*c)),
        Key::AltBackspace => "\\M-\\C-?".to_string(),
        Key::Enter => "\\C-m".to_string(),
        Key::Tab => "\\C-i".to_string(),
        Key::BackTab => "\\e[Z".to_string(),
        Key::Backspace => "\\C-?".to_string(),
        Key::Delete => "\\e[3~".to_string(),
        Key::Up => "\\e[A".to_string(),
        Key::Down => "\\e[B".to_string(),
        Key::Right => "\\e[C".to_string(),
        Key::Left => "\\e[D".to_string(),
        Key::Home => "\\e[H".to_string(),
        Key::End => "\\e[F".to_string(),
        Key::WordLeft => "\\e[1;5D".to_string(),
        Key::WordRight => "\\e[1;5C".to_string(),
        Key::PageUp => "\\e[5~".to_string(),
        Key::PageDown => "\\e[6~".to_string(),
        Key::Esc => "\\e".to_string(),
        Key::Paste(_) | Key::Other => String::new(),
    }
}

/// Collect a bracketed paste: everything up to the closing `ESC[201~`.
/// A generous inter-byte timeout bounds the collection, so a paste whose
/// end marker never arrives (a killed sender, a broken terminal) returns
/// what was received instead of hanging the editor.
fn read_paste(hooks: &dyn Hooks) -> io::Result<Key> {
    const END: &[u8] = b"\x1b[201~";
    let mut buf: Vec<u8> = Vec::new();
    while !buf.ends_with(END) {
        match read_byte_timeout(hooks, 1000)? {
            Some(b) => buf.push(b),
            None => break,
        }
    }
    if buf.ends_with(END) {
        buf.truncate(buf.len() - END.len());
    }
    Ok(Key::Paste(String::from_utf8_lossy(&buf).into_owned()))
}

fn read_key(hooks: &dyn Hooks) -> io::Result<Option<Key>> {
    let Some(b) = read_byte(hooks)? else {
        return Ok(None);
    };
    Ok(Some(match b {
        b'\r' | b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        0x7f | 0x08 => Key::Backspace,
        0x1b => {
            if !input_ready(30) {
                return Ok(Some(Key::Esc));
            }
            // The bytes after the introducer normally arrive in the same
            // burst; the bounded reads mean a half-typed sequence (a
            // human pressing ESC then `[`) abandons as `Other` instead of
            // blocking the read indefinitely.
            match read_byte(hooks)? {
                Some(b'[') => {
                    // Per ECMA-48, a CSI body is parameter bytes
                    // (0x30–0x3F: digits, `;`, and the private `< = > ?`)
                    // then intermediates (0x20–0x2F), then one final byte
                    // (0x40–0x7E). Consuming the whole grammar matters:
                    // stopping at the first non-digit used to treat `<`
                    // in an SGR mouse report (`ESC[<65;5;10M`) as the
                    // final byte, leaking `65;5;10M` into the buffer as
                    // typed text. Anything beyond the plain digits/`;`
                    // subset is swallowed whole as `Key::Other`.
                    let mut params = String::new();
                    let mut simple = true;
                    loop {
                        match read_byte_timeout(hooks, 50)? {
                            Some(c @ (b'0'..=b'9' | b';')) => params.push(c as char),
                            Some(0x20..=0x3f) => simple = false,
                            Some(final_byte @ 0x40..=0x7e) => {
                                if simple && params == "200" && final_byte == b'~' {
                                    return read_paste(hooks).map(Some);
                                }
                                return Ok(Some(if simple {
                                    csi_key(&params, final_byte)
                                } else {
                                    Key::Other
                                }));
                            }
                            _ => return Ok(Some(Key::Other)),
                        }
                    }
                }
                Some(b'O') => match read_byte_timeout(hooks, 50)? {
                    Some(b'H') => Key::Home,
                    Some(b'F') => Key::End,
                    Some(b'A') => Key::Up,
                    Some(b'B') => Key::Down,
                    Some(b'C') => Key::Right,
                    Some(b'D') => Key::Left,
                    _ => Key::Other,
                },
                Some(0x7f) => Key::AltBackspace,
                // ESC ESC: rapid double-Esc (a vi user mashing into
                // normal mode) — both used to be swallowed as `Other`.
                Some(0x1b) => Key::Esc,
                Some(c) if c.is_ascii_graphic() => Key::Alt(c as char),
                // A meta-prefixed multibyte character (Alt-ö, any
                // non-ASCII layout): assemble the whole UTF-8 sequence —
                // consuming only the lead byte left the continuations to
                // decode as garbage keys.
                Some(c) if c >= 0x80 => match read_utf8(hooks, c)? {
                    '\u{fffd}' => Key::Other,
                    ch => Key::Alt(ch),
                },
                _ => Key::Other,
            }
        }
        // The C0 bytes outside ^A..^Z (see `decode_key_bytes`); without
        // these rows, Ctrl-Space and Ctrl-\ fell through to the character
        // path and self-inserted raw NUL/FS bytes into the buffer.
        0x00 => Key::Ctrl('@'),
        0x1c => Key::Ctrl('\\'),
        0x1d => Key::Ctrl(']'),
        0x1e => Key::Ctrl('^'),
        0x1f => Key::Ctrl('_'),
        0x01..=0x1a => Key::Ctrl((b - 1 + b'a') as char),
        _ => Key::Char(read_utf8(hooks, b)?),
    }))
}

/// Terminal width in columns (fallback 80).
fn term_cols() -> usize {
    term_sys::term_cols_stdout().unwrap_or(80)
}

/// Display width of `s`, skipping ANSI SGR escape sequences — the prompt
/// and the highlighted buffer both carry them.
fn display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    let mut w = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                // CSI: skip through the final byte (0x40–0x7E).
                Some('[') => {
                    chars.next();
                    for e in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&e) {
                            break;
                        }
                    }
                }
                // OSC (window titles, OSC-8 hyperlinks): skip through
                // the BEL or `ESC \` terminator — without this, a
                // hyperlinked prompt had its whole URL counted as
                // printable width, misplacing the cursor.
                Some(']') => {
                    chars.next();
                    while let Some(e) = chars.next() {
                        if e == '\x07' {
                            break;
                        }
                        if e == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Any other two-char escape.
                _ => {
                    chars.next();
                }
            }
            continue;
        }
        w += c.width().unwrap_or(0);
    }
    w
}

/// The buffer as painted: control characters shown readline-style —
/// `^X` for C0 bytes, `^?` for DEL, `⏎` for an embedded newline (a
/// multi-line bracketed paste), a tab as four spaces. The raw buffer is
/// what Enter returns; this transform exists only so the render and its
/// cursor math never emit a raw control byte at the terminal.
fn visualize(s: &str, start_col: usize) -> String {
    visualize_marked(s, start_col, usize::MAX).0
}

/// [`visualize`], additionally reporting the display column at byte
/// offset `mark` and at the end — so the render measures the whole
/// buffer, the cursor position, and the paint in a single pass instead
/// of visualizing the buffer twice and re-measuring the output.
fn visualize_marked(s: &str, start_col: usize, mark: usize) -> (String, usize, usize) {
    let mut out = String::with_capacity(s.len());
    let mut col = start_col;
    let mut mark_col = None;
    for (i, c) in s.char_indices() {
        if mark_col.is_none() && i >= mark {
            mark_col = Some(col);
        }
        // Tabs expand to the next 8-column stop of the running display
        // offset (`start_col` is where this text begins — the prompt
        // width for the buffer), so tab-indented content lines up the
        // way every terminal renders it, not at a fixed four spaces.
        if c == '\t' {
            let pad = 8 - (col % 8);
            out.extend(std::iter::repeat_n(' ', pad));
            col += pad;
            continue;
        }
        // Track the column by measuring what was actually appended, so
        // the expansion can never drift from the cursor math.
        let before = out.len();
        push_vis_char(&mut out, c);
        col += display_width(&out[before..]);
    }
    (out, mark_col.unwrap_or(col), col)
}

/// One character of [`visualize`]'s transform — everything except tabs,
/// whose expansion is position-dependent and handled by the callers.
fn push_vis_char(out: &mut String, c: char) {
    match c {
        '\n' => out.push('⏎'),
        '\u{7f}' => out.push_str("^?"),
        c if (c as u32) < 0x20 => {
            out.push('^');
            out.push(((c as u8) ^ 0x40) as char);
        }
        c => out.push(c),
    }
}

/// [`visualize`], but passing complete SGR sequences (`ESC[…m`) through
/// untouched — applied to [`Hooks::highlight`]'s output, so the host
/// paints the *raw* buffer and the editor re-applies the control-char
/// transform around the color markup. A non-SGR escape from the hook is
/// visualized (`^[`) rather than sent to the terminal.
fn visualize_keep_sgr(s: &str, start_col: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = start_col;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            let mut body = String::new();
            let mut fin = None;
            for e in chars.by_ref() {
                if e.is_ascii_digit() || e == ';' || e == ':' {
                    body.push(e);
                } else {
                    fin = Some(e);
                    break;
                }
            }
            if fin == Some('m') {
                // Zero display width: the column doesn't advance.
                out.push_str("\x1b[");
                out.push_str(&body);
                out.push('m');
            } else {
                // Not SGR: neutralize the ESC, keep the rest as text.
                let before = out.len();
                out.push_str("^[[");
                out.push_str(&body);
                if let Some(f) = fin {
                    push_vis_char(&mut out, f);
                }
                col += display_width(&out[before..]);
            }
            continue;
        }
        if c == '\t' {
            let pad = 8 - (col % 8);
            out.extend(std::iter::repeat_n(' ', pad));
            col += pad;
            continue;
        }
        let before = out.len();
        push_vis_char(&mut out, c);
        col += display_width(&out[before..]);
    }
    out
}

/// What the previous key did — drives undo coalescing (runs of plain
/// inserts undo as one unit), kill-ring appending (consecutive kills grow
/// one ring entry), yank-pop eligibility, and M-. cycling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Insert,
    /// A single-character delete (Backspace/Delete) — runs coalesce in
    /// the undo history like insert runs do.
    Delete,
    KillFwd,
    KillBack,
    Yank,
    LastArg,
    /// A PageUp/PageDown prefix search — consecutive presses keep the
    /// anchored prefix; anything else re-anchors the next search.
    PrefixSearch,
    Undo,
    Other,
}

/// The per-`read_line` editing state.
struct LineState<'a> {
    buffer: String,
    cursor: usize, // byte offset into buffer
    prompt: &'a str,
    rprompt: &'a str,
    /// Rows the previous paint occupied, and which row the cursor was
    /// left on — the starting point for the next repaint.
    painted_rows: usize,
    painted_cursor_row: usize,
    /// The next render starts a brand-new region (first paint, after
    /// C-l, a candidate list, a host binding…): the lines of a
    /// multi-line prompt above its final line are printed then, and only
    /// then — repaints touch just the final prompt line.
    fresh_region: bool,
    /// A repaint was skipped because more input was already queued
    /// (readline's coalescing trick — see the main loop's bottom): the
    /// skipped frame would be instantly overwritten by the very next
    /// key's repaint, so painting it is pure waste while a burst is
    /// draining. `finish_line` flushes this before its cursor math runs
    /// — that math assumes the terminal's physical cursor sits exactly
    /// where `painted_rows`/`painted_cursor_row` say, which is only true
    /// immediately after a real (unskipped) render.
    render_owed: bool,
    /// How many repaints in a row have been coalesced away. Capped (see
    /// the main loop's bottom) so a very large or continuous input flood
    /// — an unbracketed paste, a runaway automation script — still shows
    /// periodic progress instead of looking frozen until the flood ends.
    coalesced_run: u32,
    /// History navigation: index into `history` (None = live line), the
    /// draft stashed when navigation started, and the anchored prefix for
    /// PageUp/PageDown prefix search.
    hist_index: Option<usize>,
    draft: String,
    prefix: String,
    /// In-session edits to recalled history entries: navigating away
    /// from a modified entry stashes it here, and coming back restores
    /// it — zsh's behavior (edits revert once the line is accepted,
    /// since this lives per `read_line`).
    hist_edits: std::collections::HashMap<usize, String>,
    /// vi mode: true when in normal mode; `vi_op` holds a pending
    /// `d`/`c`/`y` operator, `vi_find` a pending `f F t T`, `vi_count`
    /// the accumulated count, `vi_replace` a pending `r`, `last_find`
    /// the target `;`/`,` repeat.
    vi: bool,
    vi_normal: bool,
    vi_count: usize,
    vi_op: Option<char>,
    vi_find: Option<char>,
    vi_replace: bool,
    /// A pending text object: `i`/`a` typed with an operator pending,
    /// waiting for the object key (`w`/`W`).
    vi_textobj: Option<char>,
    last_find: Option<(char, char)>,
    /// Ctrl-R/Ctrl-S incremental search, when active.
    search: Option<SearchState>,
    /// Undo stack: (buffer, cursor) snapshots taken before mutations.
    undo: VecDeque<(String, usize)>,
    prev_action: Action,
    this_action: Action,
    /// Length of the current coalesced self-insert (or single-delete)
    /// undo group (capped at 20, readline's chunk size).
    edit_run: usize,
    /// Last yank's span and ring index, for M-y yank-pop.
    yank: Option<(usize, usize, usize)>,
    /// Last M-. insertion: (history index, span start, span end).
    lastarg: Option<(usize, usize, usize)>,
    /// Completion menu cycling state, armed by the candidate list and
    /// cleared by any non-Tab key.
    menu: Option<MenuState>,
    /// The `read_line_timeout` deadline, so helpers that block on their
    /// own reads (C-x chords, quoted insert) honor it too.
    deadline: Option<std::time::Instant>,
    /// The hint for the current buffer, memoized so one keystroke asks
    /// the host at most once (render + accept paths both consult it).
    hint_cache: Option<(String, Option<String>)>,
    /// The editor's readline-variable knobs, copied per read_line.
    cfg: EditorConfig,
    hooks: &'a dyn Hooks,
}

/// Repeated-Tab candidate cycling (zsh `AUTO_MENU`): the word span being
/// replaced and the candidate list captured when it was printed.
struct MenuState {
    /// Byte offset the completed word starts at.
    start: usize,
    /// Length of the replacement currently in the buffer.
    inserted: usize,
    /// Candidate currently inserted; `None` until the first cycle.
    index: Option<usize>,
    candidates: Vec<Candidate>,
}

struct SearchState {
    query: String,
    /// Index into history of the current match, if any.
    hit: Option<usize>,
    /// Search direction: C-r steps older, C-s newer.
    forward: bool,
    /// The last search operation found nothing — readline's
    /// `(failed reverse-i-search)` state.
    failed: bool,
}

/// Sanitize a word-start offset returned by a host hook (`complete`,
/// `expand_abbreviation`): past-the-cursor or mid-character offsets
/// would panic the slicing below — a hook bug must not take the host's
/// read loop down.
fn clamp_start(s: &str, start: usize, cursor: usize) -> usize {
    let mut start = start.min(cursor);
    while !s.is_char_boundary(start) {
        start -= 1;
    }
    start
}

/// Locate a C-o recall target: `i` where the entry was at capture time,
/// re-found by content when erasedups or a host history edit shifted the
/// indices in between.
fn resolve_recall(history: &[String], i: usize, expected: &str) -> Option<usize> {
    if history.get(i).map(String::as_str) == Some(expected) {
        return Some(i);
    }
    history.iter().rposition(|h| h == expected)
}

fn read_line_raw(
    ed: &mut Editor,
    prompt: &str,
    rprompt: &str,
    hooks: &dyn Hooks,
    deadline: Option<std::time::Instant>,
    initial: Option<(&str, &str)>,
) -> io::Result<ReadResult> {
    let raw = RawMode::enable()?;
    let _paste = BracketedPaste::enable();
    let cfg = ed.cfg;
    let Editor {
        history,
        kill_ring,
        bindings,
        next_recall,
        ..
    } = ed;
    let mut st = LineState {
        buffer: String::new(),
        cursor: 0,
        prompt,
        rprompt,
        painted_rows: 1,
        painted_cursor_row: 0,
        fresh_region: true,
        render_owed: false,
        coalesced_run: 0,
        hist_index: None,
        draft: String::new(),
        prefix: String::new(),
        hist_edits: std::collections::HashMap::new(),
        vi: hooks.vi_mode(),
        vi_normal: false,
        vi_count: 0,
        vi_op: None,
        vi_find: None,
        vi_replace: false,
        vi_textobj: None,
        last_find: None,
        search: None,
        undo: VecDeque::new(),
        prev_action: Action::Other,
        this_action: Action::Other,
        edit_run: 0,
        yank: None,
        lastarg: None,
        menu: None,
        deadline,
        hint_cache: None,
        cfg,
        hooks,
    };
    if let Some((left, right)) = initial {
        // An explicitly seeded line (read_line_with_initial) wins over a
        // pending C-o recall.
        st.buffer = format!("{left}{right}");
        st.cursor = left.len();
        next_recall.take();
    } else if let Some((i, expected)) = next_recall.take()
        // C-o on the previous line: pre-load the history entry after the
        // one just executed (readline `operate-and-get-next`) — located
        // by content when erasedups/host edits shifted the indices.
        && let Some(i) = resolve_recall(history, i, &expected)
    {
        st.buffer = history[i].clone();
        st.cursor = st.buffer.len();
        st.hist_index = Some(i);
    }
    render(&mut st, history)?;

    let mut cols = term_cols();
    loop {
        // Idle wait: repaint if the terminal was resized while sitting at
        // the prompt (no SIGWINCH handler — the 200ms poll tick notices).
        // The tick also gives the host a beat to fire pending signal traps
        // even when no input arrives to interrupt. The read_line_timeout
        // deadline is checked on the same tick (and between keystrokes).
        loop {
            if let Some(d) = deadline
                && std::time::Instant::now() >= d
            {
                finish_line(&mut st, history)?;
                return Ok(ReadResult::TimedOut);
            }
            if input_ready(200) {
                break;
            }
            hooks.on_interrupted_read();
            // A hook called `prepare_external_output` (e.g. a shell
            // printing a background job's completion notice) — it already
            // moved to a fresh line and wrote its own text there; this
            // just resets the paint bookkeeping so the repaint below
            // starts fresh from here instead of trying to move up through
            // rows that no longer hold what it thinks they do. No
            // `writeln!` here (unlike the raw-mode branch just below) —
            // `prepare_external_output` already did that itself.
            if EXTERNAL_OUTPUT_PENDING.with(|f| f.replace(false)) {
                st.painted_rows = 1;
                st.painted_cursor_row = 0;
                st.fresh_region = true;
                render(&mut st, history)?;
                st.render_owed = false; // just painted for real
                st.coalesced_run = 0;
            }
            // Self-healing raw mode: an external SIGTSTP + `fg` (the
            // parent shell restores *its* termios on continue), or a
            // trap/host command that ran `stty`, leaves the terminal
            // cooked — the editor would echo doubly and read
            // line-buffered until the next read_line. readline
            // re-prepares the terminal after SIGCONT; without a signal
            // handler, the tick checks and re-asserts instead.
            if let Ok(cur) = term_sys::tcgetattr_stdin()
                && !term_sys::is_raw(&cur)
            {
                raw.resume();
                st.painted_rows = 1;
                st.painted_cursor_row = 0;
                st.fresh_region = true;
                writeln!(io::stdout())?;
                render(&mut st, history)?;
                st.render_owed = false; // just painted for real
                st.coalesced_run = 0;
            }
            let now = term_cols();
            if now != cols {
                cols = now;
                render(&mut st, history)?;
                st.render_owed = false; // just painted for real
                st.coalesced_run = 0;
            }
        }

        let Some(key) = read_key(hooks)? else {
            // EOF on stdin itself. With text pending, the line is
            // returned rather than discarded — readline's rule, and what
            // the piped-stdin path already does; only an empty line is
            // end-of-input.
            finish_line(&mut st, history)?;
            return Ok(if st.buffer.is_empty() {
                ReadResult::Eof
            } else {
                ReadResult::Line(st.buffer)
            });
        };

        // Ctrl-R/Ctrl-S search intercepts everything while active.
        if st.search.is_some() {
            match handle_search_key(&mut st, key, history)? {
                SearchOutcome::Continue | SearchOutcome::Exit => {
                    render(&mut st, history)?;
                    st.render_owed = false; // just painted for real
                    st.coalesced_run = 0;
                    continue;
                }
                SearchOutcome::Accept => {
                    finish_line(&mut st, history)?;
                    return Ok(ReadResult::Line(st.buffer));
                }
                SearchOutcome::Interrupt => {
                    finish_line(&mut st, history)?;
                    return Ok(ReadResult::Interrupted);
                }
            }
        }

        let snapshot = (st.buffer.clone(), st.cursor);
        st.this_action = Action::Other;

        // Any key ends candidate cycling, unless it performs completion
        // itself (Tab by default; rebindable).
        if !key_completes(&key, bindings, st.vi && st.vi_normal) {
            st.menu = None;
        }

        let after = if st.vi && st.vi_normal {
            // vi normal mode is a fixed keymap; only the global controls
            // (line termination, search, undo, C-x chords, quoted insert,
            // `v` external edit) are shared with the binding path.
            match key {
                Key::Enter => AfterKey::Accept,
                Key::Ctrl('c') => AfterKey::Interrupted,
                Key::Ctrl('d') if st.buffer.is_empty() => AfterKey::Eof,
                Key::Ctrl('r') => {
                    start_search(&mut st, history, false);
                    AfterKey::Done
                }
                Key::Ctrl('s') => {
                    start_search(&mut st, history, true);
                    AfterKey::Done
                }
                Key::Ctrl('l') => {
                    clear_screen(&mut st)?;
                    AfterKey::Done
                }
                Key::Ctrl('_') | Key::Ctrl('z') => {
                    undo_cmd(&mut st);
                    AfterKey::Done
                }
                Key::Ctrl('x') => ctrl_x_chord(&mut st)?,
                Key::Ctrl('v') | Key::Ctrl('q') => {
                    quoted_insert(&mut st)?;
                    AfterKey::Done
                }
                Key::Tab => run_action(&mut st, EditorAction::Complete, &key, history, kill_ring)?,
                Key::BackTab => run_action(
                    &mut st,
                    EditorAction::MenuCompleteBackward,
                    &key,
                    history,
                    kill_ring,
                )?,
                Key::Paste(s) => {
                    // A bracketed paste inserts literally in normal mode
                    // too (vim, readline vi mode) — it must not run
                    // through the normal-mode keymap as fake keystrokes.
                    insert_paste(&mut st, &s);
                    AfterKey::Done
                }
                Key::Char('v') if st.vi_op.is_none() && st.vi_find.is_none() && !st.vi_replace => {
                    // vi normal-mode `v`: edit the line in $EDITOR,
                    // readline's own vi binding.
                    AfterKey::External
                }
                key => {
                    handle_vi_normal(&mut st, key, history, kill_ring);
                    AfterKey::Done
                }
            }
        } else {
            match key {
                Key::Paste(s) => {
                    // Insert the paste verbatim — no completion, no
                    // abbreviations, no history motion, no bindings.
                    insert_paste(&mut st, &s);
                    AfterKey::Done
                }
                Key::Esc if st.vi => {
                    st.vi_normal = true;
                    // vi leaves the cursor on the last inserted character.
                    if let Some(prev) = prev_char_start(&st.buffer, st.cursor) {
                        st.cursor = prev;
                    }
                    AfterKey::Done
                }
                // Emacs / vi-insert: host rebindings first, then the
                // default keymap.
                key => match bindings.iter().find(|(k, _)| *k == key) {
                    Some((_, Binding::Unbound)) => AfterKey::Done,
                    Some((_, Binding::Host(tag))) => {
                        run_host_binding(&mut st, &raw, tag, history)?;
                        AfterKey::Done
                    }
                    Some((_, Binding::Action(action))) => {
                        run_action(&mut st, *action, &key, history, kill_ring)?
                    }
                    None if key == Key::Ctrl('x') => ctrl_x_chord(&mut st)?,
                    None => match default_action(&key) {
                        Some(action) => run_action(&mut st, action, &key, history, kill_ring)?,
                        None => AfterKey::Done,
                    },
                },
            }
        };
        match after {
            AfterKey::Done => {}
            AfterKey::Accept => {
                finish_line(&mut st, history)?;
                return Ok(ReadResult::Line(st.buffer));
            }
            AfterKey::AcceptAndRecall => {
                // C-o: like Accept, but remember the entry after this one
                // so the next read_line starts on it.
                *next_recall = st
                    .hist_index
                    .and_then(|i| history.get(i + 1).map(|e| (i + 1, e.clone())));
                finish_line(&mut st, history)?;
                return Ok(ReadResult::Line(st.buffer));
            }
            AfterKey::Interrupted => {
                finish_line(&mut st, history)?;
                return Ok(ReadResult::Interrupted);
            }
            AfterKey::Eof => {
                finish_line(&mut st, history)?;
                return Ok(ReadResult::Eof);
            }
            AfterKey::External => {
                if let Some(line) = edit_in_editor(&mut st, &raw, history)? {
                    return Ok(ReadResult::Line(line));
                }
            }
        }

        record_undo(&mut st, snapshot);
        st.prev_action = st.this_action;

        // Coalesce repaints while more input is already queued
        // (readline's trick): a skipped frame would be instantly
        // overwritten by the next key's repaint anyway.
        // `input_ready(0)` is a zero-timeout poll — free when
        // `STDIN_BUF` already has bytes buffered, so this never delays
        // draining a burst — and `render_owed` guarantees the next
        // `finish_line` (Enter, C-c, a completion listing, a host
        // binding, …) forces one fresh paint first, so its
        // cursor-position math is never stale. Capped at
        // `MAX_COALESCED_RUN`: a continuous flood (a huge unbracketed
        // paste, runaway automation) would otherwise show zero visual
        // progress for as long as it lasts — indistinguishable from a
        // hang — so a paint is forced periodically regardless.
        if input_ready(0) && st.coalesced_run < MAX_COALESCED_RUN {
            st.render_owed = true;
            st.coalesced_run += 1;
        } else {
            render(&mut st, history)?;
            st.render_owed = false;
            st.coalesced_run = 0;
        }
    }
}

/// Cap on consecutive coalesced repaints — see the main loop's bottom.
const MAX_COALESCED_RUN: u32 = 200;

/// Post-key undo bookkeeping: snapshot any mutation, coalescing runs of
/// plain self-insert — and runs of single-character deletes — in groups
/// of at most 20 characters, readline's chunking: one undo neither wipes
/// an entire typed line nor replays a long rubout one character at a
/// time.
fn record_undo(st: &mut LineState, snapshot: (String, usize)) {
    if st.buffer == snapshot.0 || st.this_action == Action::Undo {
        return;
    }
    let groupable = matches!(st.this_action, Action::Insert | Action::Delete);
    let coalesce = groupable && st.this_action == st.prev_action && st.edit_run < 20;
    if !coalesce {
        st.undo.push_back(snapshot);
        while st.undo.len() > st.cfg.max_undo {
            st.undo.pop_front();
        }
    }
    st.edit_run = match (groupable, coalesce) {
        (true, true) => st.edit_run + 1,
        (true, false) => 1,
        _ => 0,
    };
}

/// Move to the end of the painted region and start a fresh terminal line,
/// so whatever runs next begins below the edit region.
fn finish_line(st: &mut LineState, history: &[String]) -> io::Result<()> {
    // The "move down to the end of the region" math below trusts
    // `painted_rows`/`painted_cursor_row` to describe exactly where the
    // terminal's physical cursor is — true only right after a render
    // that actually wrote. A coalesced burst may have deferred that
    // write; catch up here, in the one place every exit path funnels
    // through, so no call site can forget it.
    if st.render_owed {
        render(st, history)?;
        st.render_owed = false;
        st.coalesced_run = 0;
    }
    let down = st.painted_rows.saturating_sub(1 + st.painted_cursor_row);
    let mut out = io::stdout().lock();
    if down > 0 {
        write!(out, "\x1b[{down}B")?;
    }
    writeln!(out)?;
    out.flush()
}

/// Whether a key resolves to a completion action — the one case that
/// must not tear down the menu-cycling state before dispatch.
fn key_completes(key: &Key, bindings: &[(Key, Binding)], vi_normal: bool) -> bool {
    if vi_normal {
        // vi normal mode's keymap is fixed: Tab completes and Shift-Tab
        // cycles backward there.
        return matches!(key, Key::Tab | Key::BackTab);
    }
    let action = match bindings.iter().find(|(k, _)| k == key) {
        Some((_, Binding::Action(a))) => Some(*a),
        Some(_) => None,
        None => default_action(key),
    };
    matches!(
        action,
        Some(
            EditorAction::Complete
                | EditorAction::MenuComplete
                | EditorAction::MenuCompleteBackward
        )
    )
}

/// What the main loop does after a key's action ran: nothing, end the
/// read (three ways), or hand the line to the external editor (which
/// needs the loop's `RawMode` handle).
enum AfterKey {
    Done,
    Accept,
    /// Accept, and pre-load the next history entry for the next
    /// `read_line` (C-o, `operate-and-get-next`).
    AcceptAndRecall,
    Interrupted,
    Eof,
    External,
}

/// Insert pasted text at the cursor, normalizing line endings — shared by
/// the emacs/vi-insert and vi-normal paste paths.
fn insert_paste(st: &mut LineState, s: &str) {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    st.buffer.insert_str(st.cursor, &s);
    st.cursor += s.len();
}

/// Enter C-r / C-s incremental search.
fn start_search(st: &mut LineState, history: &[String], forward: bool) {
    st.search = Some(SearchState {
        query: String::new(),
        // Search from the current history position: after recalling a
        // line with Up, C-r continues backward from there (readline),
        // not from the newest entry. Only an *unedited* recall seeds the
        // position — exiting a seeded search rewrites the buffer from
        // history, which must never clobber the user's edits.
        hit: st
            .hist_index
            .filter(|&i| history.get(i) == Some(&st.buffer)),
        forward,
        failed: false,
    });
}

/// C-l: clear the screen; the next render repaints at the top.
fn clear_screen(st: &mut LineState) -> io::Result<()> {
    write!(io::stdout(), "\x1b[2J\x1b[H")?;
    st.painted_rows = 1;
    st.painted_cursor_row = 0;
    st.fresh_region = true;
    Ok(())
}

/// Wait for the next byte, honoring the `read_line_timeout` deadline and
/// giving the host its idle beat — for the helpers that read their own
/// follow-up key (C-x chords, quoted insert). `false` means the deadline
/// passed; the caller gives up and the main loop returns `TimedOut`.
fn wait_for_key(st: &LineState) -> io::Result<bool> {
    loop {
        if let Some(d) = st.deadline
            && std::time::Instant::now() >= d
        {
            return Ok(false);
        }
        if input_ready(200) {
            return Ok(true);
        }
        st.hooks.on_interrupted_read();
    }
}

/// The readline C-x chords supported: C-x C-e (edit the line in
/// $EDITOR) and C-x C-u (undo).
fn ctrl_x_chord(st: &mut LineState) -> io::Result<AfterKey> {
    if !wait_for_key(st)? {
        return Ok(AfterKey::Done); // deadline: the main loop times out
    }
    Ok(match read_key(st.hooks)? {
        Some(Key::Ctrl('e')) => AfterKey::External,
        Some(Key::Ctrl('u')) => {
            undo_cmd(st);
            AfterKey::Done
        }
        _ => AfterKey::Done,
    })
}

/// Ring the terminal bell per the configured style.
fn bell(style: BellStyle) -> io::Result<()> {
    match style {
        BellStyle::None => return Ok(()),
        BellStyle::Audible => write!(io::stdout(), "\x07")?,
        // Reverse-video flip — the flash without terminfo's `flash`.
        BellStyle::Visible => {
            // Hold the reverse-video flash long enough to actually
            // render (terminfo's `flash` pauses too — set-then-unset in
            // one write can show zero frames). The poll keeps input
            // responsive: a keystroke just ends the flash early.
            write!(io::stdout(), "\x1b[?5h")?;
            io::stdout().flush()?;
            let _ = input_ready(80);
            write!(io::stdout(), "\x1b[?5l")?;
        }
    }
    io::stdout().flush()
}

/// A `bind_host` key: suspend raw mode, hand the line and cursor to the
/// host (bash's `bind -x`, `READLINE_LINE`/`READLINE_POINT`), take back
/// whatever it wrote, and repaint on a fresh region.
fn run_host_binding(
    st: &mut LineState,
    raw: &RawMode,
    tag: &str,
    history: &[String],
) -> io::Result<()> {
    finish_line(st, history)?;
    raw.suspend();
    let mut line = std::mem::take(&mut st.buffer);
    let mut cursor = st.cursor;
    st.hooks.host_binding(tag, &mut line, &mut cursor);
    raw.resume();
    st.buffer = line;
    st.cursor = cursor.min(st.buffer.len());
    while !st.buffer.is_char_boundary(st.cursor) {
        st.cursor -= 1;
    }
    st.painted_rows = 1;
    st.painted_cursor_row = 0;
    st.fresh_region = true;
    Ok(())
}

/// Execute one named action — the emacs (and vi-insert) command set.
/// `key` supplies the character for `SelfInsert`.
fn run_action(
    st: &mut LineState,
    action: EditorAction,
    key: &Key,
    history: &[String],
    ring: &mut VecDeque<String>,
) -> io::Result<AfterKey> {
    match action {
        EditorAction::AcceptLine => return Ok(AfterKey::Accept),
        EditorAction::OperateAndGetNext => return Ok(AfterKey::AcceptAndRecall),
        EditorAction::Interrupt => return Ok(AfterKey::Interrupted),
        EditorAction::DeleteCharOrEof if st.buffer.is_empty() => return Ok(AfterKey::Eof),
        EditorAction::EditAndExecuteCommand => return Ok(AfterKey::External),
        EditorAction::SelfInsert => {
            let &Key::Char(c) = key else {
                return Ok(AfterKey::Done); // only character keys self-insert
            };
            // Abbreviations (fish-style): a space after one defined in
            // command position rewrites it in place first.
            if c == ' '
                && let Some((start, expansion)) =
                    st.hooks.expand_abbreviation(&st.buffer, st.cursor)
            {
                let start = clamp_start(&st.buffer, start, st.cursor);
                st.buffer.replace_range(start..st.cursor, &expansion);
                st.cursor = start + expansion.len();
            }
            insert_char(st, c);
            st.this_action = Action::Insert;
        }
        EditorAction::BackwardDeleteChar => {
            if let Some(prev) = prev_char_start(&st.buffer, st.cursor) {
                st.buffer.replace_range(prev..st.cursor, "");
                st.cursor = prev;
                st.this_action = Action::Delete;
            }
        }
        EditorAction::DeleteChar | EditorAction::DeleteCharOrEof => {
            if let Some(next) = next_char_end(&st.buffer, st.cursor) {
                st.buffer.replace_range(st.cursor..next, "");
                st.this_action = Action::Delete;
            }
        }
        EditorAction::BackwardChar => {
            if let Some(prev) = prev_char_start(&st.buffer, st.cursor) {
                st.cursor = prev;
            }
        }
        EditorAction::ForwardChar => {
            // At end of line, the right arrow accepts the history hint.
            if st.cursor == st.buffer.len() {
                if let Some(hint) = cached_hint(st, history) {
                    st.buffer.push_str(&hint);
                    st.cursor = st.buffer.len();
                }
            } else if let Some(next) = next_char_end(&st.buffer, st.cursor) {
                st.cursor = next;
            }
        }
        EditorAction::BeginningOfLine => st.cursor = 0,
        EditorAction::EndOfLine => {
            // End at end-of-line also accepts the hint (fish's behavior).
            if st.cursor == st.buffer.len()
                && let Some(hint) = cached_hint(st, history)
            {
                st.buffer.push_str(&hint);
            }
            st.cursor = st.buffer.len();
        }
        EditorAction::BackwardWord => st.cursor = word_back_alnum(&st.buffer, st.cursor),
        EditorAction::ForwardWord => {
            // At end of line, accept one word of the history hint
            // (fish's forward-word on an autosuggestion).
            if st.cursor == st.buffer.len() {
                if let Some(hint) = cached_hint(st, history) {
                    let take = word_forward_alnum(&hint, 0);
                    st.buffer.push_str(&hint[..take]);
                    st.cursor = st.buffer.len();
                }
            } else {
                st.cursor = word_forward_alnum(&st.buffer, st.cursor);
            }
        }
        EditorAction::KillLine => kill_span(st, ring, st.cursor, st.buffer.len(), true),
        EditorAction::KillWholeLine => kill_span(st, ring, 0, st.buffer.len(), true),
        EditorAction::DeleteHorizontalSpace => {
            // readline: delete the spaces and tabs around point — a plain
            // delete, not a kill (nothing enters the ring).
            let is_blank = |c: char| c == ' ' || c == '\t';
            let start = st.buffer[..st.cursor]
                .rfind(|c| !is_blank(c))
                .map(|i| next_char_end(&st.buffer, i).unwrap_or(i))
                .unwrap_or(0);
            let end = st.buffer[st.cursor..]
                .find(|c| !is_blank(c))
                .map(|i| st.cursor + i)
                .unwrap_or(st.buffer.len());
            if start < end {
                st.buffer.replace_range(start..end, "");
                st.cursor = start;
            }
        }
        EditorAction::UnixLineDiscard => kill_span(st, ring, 0, st.cursor, false),
        EditorAction::UnixWordRubout => {
            // unix-word-rubout: whitespace-delimited, unlike M-Backspace.
            let start = word_back(&st.buffer, st.cursor);
            kill_span(st, ring, start, st.cursor, false);
        }
        EditorAction::KillWord => {
            let end = word_forward_alnum(&st.buffer, st.cursor);
            kill_span(st, ring, st.cursor, end, true);
        }
        EditorAction::BackwardKillWord => {
            let start = word_back_alnum(&st.buffer, st.cursor);
            kill_span(st, ring, start, st.cursor, false);
        }
        EditorAction::Yank => yank(st, ring),
        EditorAction::YankPop => yank_pop(st, ring),
        EditorAction::TransposeChars => transpose(st),
        EditorAction::TransposeWords => transpose_words(st),
        EditorAction::UpcaseWord => case_word(st, CaseOp::Upper),
        EditorAction::DowncaseWord => case_word(st, CaseOp::Lower),
        EditorAction::CapitalizeWord => case_word(st, CaseOp::Capital),
        EditorAction::Undo => undo_cmd(st),
        EditorAction::RevertLine => {
            // readline revert-line: undo every edit to this line at once.
            if let Some((buf, cur)) = st.undo.front().cloned() {
                st.cursor = cur.min(buf.len());
                st.buffer = buf;
                st.undo.clear();
            }
            st.this_action = Action::Undo; // don't snapshot the revert
        }
        EditorAction::InsertLastArgument => insert_last_arg(st, history),
        EditorAction::BeginningOfHistory => history_first(st, history),
        EditorAction::EndOfHistory => history_last(st, history),
        EditorAction::PreviousHistory => history_prev(st, history),
        EditorAction::NextHistory => history_next(st, history),
        EditorAction::HistorySearchBackward => history_prefix_prev(st, history),
        EditorAction::HistorySearchForward => history_prefix_next(st, history),
        EditorAction::ReverseSearchHistory => start_search(st, history, false),
        EditorAction::ForwardSearchHistory => start_search(st, history, true),
        EditorAction::ClearScreen => clear_screen(st)?,
        EditorAction::Complete => {
            if st.menu.is_some() {
                menu_next(st);
            } else if st.cfg.menu_complete {
                menu_complete_start(st)?;
            } else {
                complete_at_cursor(st, history)?;
            }
        }
        EditorAction::MenuComplete => {
            if st.menu.is_some() {
                menu_next(st);
            } else {
                menu_complete_start(st)?;
            }
        }
        EditorAction::MenuCompleteBackward => {
            // Shift-Tab: step the cycle backward; cold, it starts on the
            // *last* candidate (readline `menu-complete-backward`).
            if st.menu.is_none() && !menu_arm(st)? {
                return Ok(AfterKey::Done);
            }
            menu_step(st, true);
        }
        EditorAction::PossibleCompletions => {
            // M-?: list the candidates without touching the buffer — no
            // LCP insertion, no menu cycling armed.
            let (_, mut candidates) = st.hooks.complete(&st.buffer, st.cursor);
            if candidates.is_empty() {
                bell(st.cfg.bell)?;
            } else {
                candidates.sort_by(|a, b| a.display.cmp(&b.display));
                list_candidates(st, &candidates, history)?;
            }
        }
        EditorAction::InsertCompletions => {
            // M-*: insert every match, space-separated (readline).
            let (start, mut candidates) = st.hooks.complete(&st.buffer, st.cursor);
            let start = clamp_start(&st.buffer, start, st.cursor);
            if candidates.is_empty() {
                bell(st.cfg.bell)?;
            } else {
                candidates.sort_by(|a, b| a.display.cmp(&b.display));
                let joined = candidates
                    .iter()
                    .map(|c| c.replacement.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                st.buffer.replace_range(start..st.cursor, &joined);
                st.cursor = start + joined.len();
            }
        }
        EditorAction::CharacterSearch => character_search(st, false)?,
        EditorAction::CharacterSearchBackward => character_search(st, true)?,
        EditorAction::Abort => bell(st.cfg.bell)?,
        EditorAction::QuotedInsert => quoted_insert(st)?,
    }
    Ok(AfterKey::Done)
}

/// vi normal mode: counts, the `d`/`c`/`y` operators over motions
/// (`h l 0 ^ $ w W b B e E f F t T ; , %` and doubled `dd cc yy`) and
/// the `iw`/`aw` text objects, edits `x X D C s S Y r ~ p P u`, inserts
/// `i I a A`, history `k j G`. See the README for what's deliberately
/// not modeled (registers, `.` repeat, `/` search).
fn handle_vi_normal(st: &mut LineState, key: Key, history: &[String], ring: &mut VecDeque<String>) {
    // A pending `r`: the next character replaces the `count` characters
    // under the cursor (`3rx` → `xxx`); like vim, it fails outright when
    // fewer than `count` remain.
    if st.vi_replace {
        st.vi_replace = false;
        let n = st.vi_count.max(1);
        st.vi_count = 0;
        if let Key::Char(c) = key {
            let end = fwd_n(&st.buffer, st.cursor, n);
            if st.buffer[st.cursor..end].chars().count() == n {
                let replacement: String = std::iter::repeat_n(c, n).collect();
                st.buffer.replace_range(st.cursor..end, &replacement);
                // The cursor lands on the last replaced character.
                st.cursor += c.len_utf8() * (n - 1);
            } else {
                let _ = bell(st.cfg.bell); // not enough characters left
            }
        }
        return;
    }
    // A pending text object (`di`/`ca`/`yi` …): the next key names the
    // object; only `w`/`W` are modeled.
    if let Some(obj_kind) = st.vi_textobj.take() {
        st.vi_count = 0;
        let op = st.vi_op.take();
        if let (Key::Char('w' | 'W'), Some(op)) = (&key, op) {
            match vi_word_object(&st.buffer, st.cursor, obj_kind == 'a') {
                Some((s, e)) => match op {
                    'd' => kill_span(st, ring, s, e, true),
                    'c' => {
                        kill_span(st, ring, s, e, true);
                        st.vi_normal = false;
                    }
                    _ => {
                        push_ring(ring, st.buffer[s..e].to_string(), st.cfg.max_kill_ring);
                        st.cursor = s;
                    }
                },
                None => {
                    let _ = bell(st.cfg.bell);
                }
            }
        }
        return;
    }
    // A pending `f F t T`: the next character is the find target.
    if let Some(kind) = st.vi_find.take() {
        let n = st.vi_count.max(1);
        st.vi_count = 0;
        if let Key::Char(c) = key {
            st.last_find = Some((kind, c));
            let mut hit: Option<(usize, bool)> = None;
            let mut from = st.cursor;
            for _ in 0..n {
                match vi_find_target(&st.buffer, from, kind, c) {
                    Some((t, inc)) => {
                        hit = Some((t, inc));
                        from = t;
                    }
                    None => {
                        hit = None;
                        break;
                    }
                }
            }
            match (hit, st.vi_op.take()) {
                (Some((t, inc)), Some(op)) => vi_apply_op(st, ring, op, t, inc),
                (Some((t, _)), None) => st.cursor = t,
                (None, _) => {
                    let _ = bell(st.cfg.bell); // no such character on the line
                }
            }
        } else {
            st.vi_op = None;
        }
        return;
    }
    // Count accumulation (`0` is a motion unless a count has started).
    if let Key::Char(c @ '0'..='9') = key
        && (c != '0' || st.vi_count > 0)
    {
        st.vi_count = st.vi_count * 10 + (c as usize - '0' as usize);
        return;
    }

    let n = st.vi_count.max(1);

    // Motions: resolve to a target position (and operator inclusivity).
    let motion: Option<(usize, bool)> = match &key {
        Key::Char('h') | Key::Left | Key::Backspace => {
            Some((back_n(&st.buffer, st.cursor, n), false))
        }
        Key::Char('l' | ' ') | Key::Right => Some((fwd_n(&st.buffer, st.cursor, n), false)),
        Key::Char('0') | Key::Home => Some((0, false)),
        Key::Char('^') => Some((first_nonblank(&st.buffer), false)),
        Key::Char('$') | Key::End => Some((st.buffer.len(), false)),
        Key::Char('w' | 'W') => {
            // vi quirk: `cw` behaves like `ce`.
            if st.vi_op == Some('c') {
                Some((apply_n(&st.buffer, st.cursor, n, vi_word_end), true))
            } else {
                Some((apply_n(&st.buffer, st.cursor, n, vi_word_fwd), false))
            }
        }
        Key::Char('b' | 'B') => Some((apply_n(&st.buffer, st.cursor, n, vi_word_back), false)),
        Key::Char('e' | 'E') => Some((apply_n(&st.buffer, st.cursor, n, vi_word_end), true)),
        Key::Char(k @ ('f' | 'F' | 't' | 'T')) => {
            st.vi_find = Some(*k);
            return;
        }
        Key::Char(sc @ (';' | ',')) => st.last_find.and_then(|(kind, target)| {
            let k = if *sc == ',' { invert_find(kind) } else { kind };
            vi_find_target(&st.buffer, st.cursor, k, target)
        }),
        // `%`: to the bracket matching the one at (or after) the cursor;
        // inclusive under an operator in both directions, like vim.
        Key::Char('%') => vi_match_bracket(&st.buffer, st.cursor).map(|t| (t, true)),
        _ => None,
    };
    if let Some((target, inclusive)) = motion {
        st.vi_count = 0;
        if let Some(op) = st.vi_op.take() {
            vi_apply_op(st, ring, op, target, inclusive);
        } else {
            st.cursor = target.min(st.buffer.len());
        }
        return;
    }

    match key {
        Key::Char(o @ ('d' | 'c' | 'y')) => {
            if st.vi_op == Some(o) {
                // dd / cc / yy: linewise.
                st.vi_op = None;
                match o {
                    'd' => kill_span(st, ring, 0, st.buffer.len(), true),
                    'c' => {
                        kill_span(st, ring, 0, st.buffer.len(), true);
                        st.vi_normal = false;
                    }
                    _ => {
                        if !st.buffer.is_empty() {
                            push_ring(ring, st.buffer.clone(), st.cfg.max_kill_ring);
                        }
                        st.cursor = 0;
                    }
                }
            } else {
                st.vi_op = Some(o);
                return; // keep any count for the motion
            }
        }
        Key::Char('x') => {
            let end = fwd_n(&st.buffer, st.cursor, n);
            kill_span(st, ring, st.cursor, end, true);
        }
        Key::Char('X') => {
            let start = back_n(&st.buffer, st.cursor, n);
            kill_span(st, ring, start, st.cursor, false);
        }
        Key::Char('s') => {
            let end = fwd_n(&st.buffer, st.cursor, n);
            kill_span(st, ring, st.cursor, end, true);
            st.vi_normal = false;
        }
        Key::Char('D') => kill_span(st, ring, st.cursor, st.buffer.len(), true),
        Key::Char('C') => {
            kill_span(st, ring, st.cursor, st.buffer.len(), true);
            st.vi_normal = false;
        }
        Key::Char('S') => {
            kill_span(st, ring, 0, st.buffer.len(), true);
            st.vi_normal = false;
        }
        Key::Char('Y') => {
            if !st.buffer.is_empty() {
                push_ring(ring, st.buffer.clone(), st.cfg.max_kill_ring);
            }
        }
        Key::Char('r') => {
            st.vi_replace = true;
            return;
        }
        Key::Char('~') => {
            for _ in 0..n {
                let Some(next) = next_char_end(&st.buffer, st.cursor) else {
                    break;
                };
                let flipped: String = st.buffer[st.cursor..next]
                    .chars()
                    .flat_map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().collect::<Vec<_>>()
                        } else {
                            c.to_uppercase().collect()
                        }
                    })
                    .collect();
                let len = flipped.len();
                st.buffer.replace_range(st.cursor..next, &flipped);
                st.cursor += len;
            }
        }
        Key::Char('p') => {
            // A count pastes that many copies (vim `3p`).
            if let Some(text) = ring.back()
                && !text.is_empty()
            {
                let text = text.repeat(n);
                let at = next_char_end(&st.buffer, st.cursor).unwrap_or(st.cursor);
                st.buffer.insert_str(at, &text);
                st.cursor = prev_char_start(&st.buffer, at + text.len()).unwrap_or(at);
            }
        }
        Key::Char('P') => {
            if let Some(text) = ring.back()
                && !text.is_empty()
            {
                let text = text.repeat(n);
                let at = st.cursor;
                st.buffer.insert_str(at, &text);
                st.cursor = prev_char_start(&st.buffer, at + text.len()).unwrap_or(at);
            }
        }
        Key::Char('u') => undo_cmd(st),
        // With an operator pending, `i`/`a` start a text object (`diw`,
        // `caw`, `yiw` …) instead of entering insert mode.
        Key::Char(t @ ('i' | 'a')) if st.vi_op.is_some() => {
            st.vi_textobj = Some(t);
            return; // keep the operator (and any count) for the object
        }
        Key::Char('i') => st.vi_normal = false,
        Key::Char('I') => {
            st.cursor = 0;
            st.vi_normal = false;
        }
        Key::Char('a') => {
            if let Some(next) = next_char_end(&st.buffer, st.cursor) {
                st.cursor = next;
            }
            st.vi_normal = false;
        }
        Key::Char('A') => {
            st.cursor = st.buffer.len();
            st.vi_normal = false;
        }
        Key::Char('k') | Key::Up => history_prev(st, history),
        Key::Char('j') | Key::Down => history_next(st, history),
        // `G`: with a count, fetch history entry N (1-based); without,
        // the most recent entry (vim's last line; readline
        // `vi-fetch-history`).
        Key::Char('G') => {
            if history.is_empty() {
                let _ = bell(st.cfg.bell);
            } else {
                let idx = if st.vi_count > 0 {
                    (st.vi_count - 1).min(history.len() - 1)
                } else {
                    history.len() - 1
                };
                stash_hist_edit(st, history);
                if st.hist_index.is_none() {
                    st.draft = st.buffer.clone();
                }
                st.hist_index = Some(idx);
                st.buffer = hist_entry(st, history, idx);
                st.cursor = st.buffer.len();
            }
        }
        Key::Delete => {
            // The Delete key is `x` (vim): the text goes into the ring
            // instead of vanishing.
            let end = fwd_n(&st.buffer, st.cursor, n);
            kill_span(st, ring, st.cursor, end, true);
        }
        _ => {}
    }
    st.vi_count = 0;
    st.vi_op = None; // an unrecognized key cancels a pending operator
}

/// Apply a vi operator over the span from the cursor to `target`.
fn vi_apply_op(
    st: &mut LineState,
    ring: &mut VecDeque<String>,
    op: char,
    target: usize,
    inclusive: bool,
) {
    let (s, e) = if target >= st.cursor {
        let e = if inclusive {
            next_char_end(&st.buffer, target).unwrap_or(st.buffer.len())
        } else {
            target
        };
        (st.cursor, e)
    } else {
        // A backward inclusive motion (`%`) takes the cursor's own
        // character too — vim's `d%` deletes both brackets either way.
        let e = if inclusive {
            next_char_end(&st.buffer, st.cursor).unwrap_or(st.buffer.len())
        } else {
            st.cursor
        };
        (target, e)
    };
    if s >= e {
        if op == 'c' {
            st.vi_normal = false;
        }
        return;
    }
    match op {
        'd' => kill_span(st, ring, s, e, target >= st.cursor),
        'c' => {
            kill_span(st, ring, s, e, target >= st.cursor);
            st.vi_normal = false;
        }
        _ => {
            push_ring(ring, st.buffer[s..e].to_string(), st.cfg.max_kill_ring);
            st.cursor = s;
        }
    }
}

fn invert_find(kind: char) -> char {
    match kind {
        'f' => 'F',
        'F' => 'f',
        't' => 'T',
        _ => 't',
    }
}

/// Resolve `f F t T` to a target position (and operator inclusivity).
fn vi_find_target(s: &str, pos: usize, kind: char, target: char) -> Option<(usize, bool)> {
    match kind {
        'f' | 't' => {
            let from = next_char_end(s, pos)?;
            let found = s[from..]
                .char_indices()
                .find(|&(_, c)| c == target)
                .map(|(i, _)| from + i)?;
            let t = if kind == 't' {
                prev_char_start(s, found)?
            } else {
                found
            };
            if t <= pos { None } else { Some((t, true)) }
        }
        _ => {
            let found = s[..pos]
                .char_indices()
                .rev()
                .find(|&(_, c)| c == target)
                .map(|(i, _)| i)?;
            let t = if kind == 'T' {
                next_char_end(s, found)?
            } else {
                found
            };
            if t >= pos { None } else { Some((t, false)) }
        }
    }
}

enum SearchOutcome {
    Continue,
    Accept,
    Exit,
    /// C-c during the search: abort the whole read (bash prints `^C`
    /// and gives a fresh prompt), not just the search.
    Interrupt,
}

/// Ctrl-R/Ctrl-S incremental search (bash's `(reverse-i-search)`); C-r
/// steps to older matches, C-s to newer ones.
fn handle_search_key(
    st: &mut LineState,
    key: Key,
    history: &[String],
) -> io::Result<SearchOutcome> {
    let ci = st.cfg.search_ignore_case;
    let search = st.search.as_mut().expect("search active");
    match key {
        Key::Enter => {
            let (hit, query) = (search.hit, search.query.clone());
            if let Some(hit) = hit {
                stash_hist_edit(st, history);
                // The found entry becomes the history position, so a
                // following Up walks backward from here (readline), and
                // it re-anchors on the original text.
                st.hist_edits.remove(&hit);
                st.hist_index = Some(hit);
                st.buffer = history[hit].clone();
                // The cursor lands on the match, like readline's point.
                st.cursor = match_pos(&st.buffer, &query, ci).unwrap_or(st.buffer.len());
            }
            st.search = None;
            return Ok(SearchOutcome::Accept);
        }
        Key::Ctrl('g') | Key::Esc => {
            // Abort: back to the pre-search line, with readline's bell.
            st.search = None;
            bell(st.cfg.bell)?;
            return Ok(SearchOutcome::Exit);
        }
        Key::Ctrl('c') => {
            st.search = None;
            return Ok(SearchOutcome::Interrupt);
        }
        Key::Ctrl('r') => {
            // Next older match.
            search.forward = false;
            let below = search.hit.unwrap_or(history.len());
            let found = find_match(history, &search.query, below, ci);
            search.failed = found.is_none() && !search.query.is_empty();
            search.hit = found.or(search.hit);
        }
        Key::Ctrl('s') => {
            // Next newer match.
            search.forward = true;
            let above = search.hit.map(|h| h + 1).unwrap_or(0);
            let found = find_match_fwd(history, &search.query, above, ci);
            search.failed = found.is_none() && !search.query.is_empty();
            search.hit = found.or(search.hit);
        }
        Key::Backspace => {
            search.query.pop();
            // Stay position-disciplined like the Char arm: re-search
            // at-or-before the current hit instead of teleporting to
            // the newest match when the query shrinks.
            let limit = search.hit.map(|h| h + 1).unwrap_or(history.len());
            search.hit = find_match(history, &search.query, limit, ci);
            search.failed = search.hit.is_none() && !search.query.is_empty();
        }
        Key::Char(c) => {
            search.query.push(c);
            // Search at-or-before the current position (readline keeps
            // the match as near as possible); on a miss, keep showing
            // the last match under the "failed" label.
            let limit = search.hit.map(|h| h + 1).unwrap_or(history.len());
            let found = find_match(history, &search.query, limit, ci);
            search.failed = found.is_none();
            search.hit = found.or(search.hit);
        }
        Key::Paste(s) => {
            // A paste types into the query (bash): pasting an error
            // message to find it in history must not exit the search
            // and drop the text. Entries are single-line, so newlines
            // become spaces.
            let s = s.replace(['\r', '\n'], " ");
            search.query.push_str(s.trim_end());
            let limit = search.hit.map(|h| h + 1).unwrap_or(history.len());
            let found = find_match(history, &search.query, limit, ci);
            search.failed = found.is_none();
            search.hit = found.or(search.hit);
        }
        _ => {
            // Any other key: keep the match as the edit buffer and leave
            // search mode, the cursor on the matched text (readline
            // leaves point at the found location — you searched for it
            // to edit it). The hit becomes the history position.
            let (hit, query) = (search.hit, search.query.clone());
            if let Some(hit) = hit {
                stash_hist_edit(st, history);
                st.hist_edits.remove(&hit);
                st.hist_index = Some(hit);
                st.buffer = history[hit].clone();
                st.cursor = match_pos(&st.buffer, &query, ci).unwrap_or(st.buffer.len());
            }
            st.search = None;
            return Ok(SearchOutcome::Exit);
        }
    }
    if st.search.as_ref().is_some_and(|s| s.failed) {
        bell(st.cfg.bell)?;
    }
    Ok(SearchOutcome::Continue)
}

/// Byte offset of the first occurrence of `needle` in `hay`, matched the
/// same way the search matched it — where the cursor lands when a search
/// is accepted or exited.
fn match_pos(hay: &str, needle: &str, ignore_case: bool) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    if !ignore_case {
        return hay.find(needle);
    }
    // Case-insensitive: probe every char boundary (lowercasing can change
    // byte lengths, so a lowercase `find` offset wouldn't map back).
    hay.char_indices()
        .map(|(i, _)| i)
        .find(|&i| starts_with_match(&hay[i..], needle, true))
}

/// Case-folded prefix comparison with no allocation — the ci matchers
/// run against every history entry on every search keystroke, where a
/// `to_lowercase()` per entry was two `String`s each.
fn starts_with_fold(hay: &str, prefix: &str) -> bool {
    let mut h = hay.chars().flat_map(char::to_lowercase);
    prefix
        .chars()
        .flat_map(char::to_lowercase)
        .all(|p| h.next() == Some(p))
}

/// Substring match, optionally case-insensitive (readline 8.1's
/// `search-ignore-case`).
fn contains_match(hay: &str, needle: &str, ignore_case: bool) -> bool {
    if !ignore_case {
        return hay.contains(needle);
    }
    if needle.is_empty() {
        return true;
    }
    hay.char_indices()
        .any(|(i, _)| starts_with_fold(&hay[i..], needle))
}

/// Prefix match, optionally case-insensitive — the prefix-search twin of
/// [`contains_match`].
fn starts_with_match(hay: &str, prefix: &str, ignore_case: bool) -> bool {
    if ignore_case {
        starts_with_fold(hay, prefix)
    } else {
        hay.starts_with(prefix)
    }
}

/// Most recent history entry (strictly before `below`) containing `query`.
fn find_match(history: &[String], query: &str, below: usize, ignore_case: bool) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    history[..below.min(history.len())]
        .iter()
        .rposition(|h| contains_match(h, query, ignore_case))
}

/// Earliest history entry at or after `from` containing `query`.
fn find_match_fwd(
    history: &[String],
    query: &str,
    from: usize,
    ignore_case: bool,
) -> Option<usize> {
    if query.is_empty() || from >= history.len() {
        return None;
    }
    history[from..]
        .iter()
        .position(|h| contains_match(h, query, ignore_case))
        .map(|p| p + from)
}

fn insert_char(st: &mut LineState, c: char) {
    st.buffer.insert(st.cursor, c);
    st.cursor += c.len_utf8();
}

fn prev_char_start(s: &str, pos: usize) -> Option<usize> {
    s[..pos].char_indices().next_back().map(|(i, _)| i)
}

fn next_char_end(s: &str, pos: usize) -> Option<usize> {
    s[pos..].chars().next().map(|c| pos + c.len_utf8())
}

fn fwd_n(s: &str, pos: usize, n: usize) -> usize {
    let mut p = pos;
    for _ in 0..n {
        match next_char_end(s, p) {
            Some(q) => p = q,
            None => break,
        }
    }
    p
}

fn back_n(s: &str, pos: usize, n: usize) -> usize {
    let mut p = pos;
    for _ in 0..n {
        match prev_char_start(s, p) {
            Some(q) => p = q,
            None => break,
        }
    }
    p
}

fn apply_n(s: &str, pos: usize, n: usize, f: fn(&str, usize) -> usize) -> usize {
    let mut p = pos;
    for _ in 0..n {
        p = f(s, p);
    }
    p
}

fn first_nonblank(s: &str) -> usize {
    s.char_indices()
        .find(|&(_, c)| !c.is_whitespace())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Start of the word before `pos` (whitespace-delimited — C-w's
/// unix-word-rubout and the vi big-word flavor).
fn word_back(s: &str, pos: usize) -> usize {
    let before = &s[..pos];
    let trimmed = before.trim_end();
    match trimmed.rfind(char::is_whitespace) {
        Some(i) => i + 1,
        None => 0,
    }
}

/// Start of the alphanumeric word before `pos` (readline's M-b flavor).
/// A plain reverse walk — these motions run per keystroke, so no
/// collecting the whole line into a `Vec` first.
fn word_back_alnum(s: &str, pos: usize) -> usize {
    let mut boundary = pos;
    let mut it = s[..pos].char_indices().rev();
    let mut cur = it.next();
    while let Some((i, c)) = cur {
        if c.is_alphanumeric() {
            break;
        }
        boundary = i;
        cur = it.next();
    }
    while let Some((i, c)) = cur {
        if !c.is_alphanumeric() {
            break;
        }
        boundary = i;
        cur = it.next();
    }
    boundary
}

/// End of the alphanumeric word after `pos` (readline's M-f flavor).
fn word_forward_alnum(s: &str, pos: usize) -> usize {
    let mut it = s[pos..].char_indices().peekable();
    while it.peek().is_some_and(|&(_, c)| !c.is_alphanumeric()) {
        it.next();
    }
    while it.peek().is_some_and(|&(_, c)| c.is_alphanumeric()) {
        it.next();
    }
    it.peek().map(|&(i, _)| pos + i).unwrap_or(s.len())
}

/// vi small-word character class: whitespace / word (alnum + `_`) /
/// punctuation — each non-blank run of one class is a word.
fn vi_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

/// vi `w`: start of the next small word.
fn vi_word_fwd(s: &str, pos: usize) -> usize {
    let mut it = s[pos..].char_indices().peekable();
    let Some(&(_, c0)) = it.peek() else {
        return s.len();
    };
    let cls = vi_class(c0);
    if cls != 0 {
        while let Some(&(_, c)) = it.peek() {
            if vi_class(c) == cls {
                it.next();
            } else {
                break;
            }
        }
    }
    while let Some(&(_, c)) = it.peek() {
        if vi_class(c) == 0 {
            it.next();
        } else {
            break;
        }
    }
    it.peek().map(|&(i, _)| pos + i).unwrap_or(s.len())
}

/// vi `b`: start of the small word before `pos` — a reverse walk, no
/// per-keystroke `Vec` of the line.
fn vi_word_back(s: &str, pos: usize) -> usize {
    let mut it = s[..pos].char_indices().rev();
    let mut cur = it.next();
    while let Some((_, c)) = cur {
        if vi_class(c) != 0 {
            break;
        }
        cur = it.next();
    }
    let Some((mut boundary, c0)) = cur else {
        return 0;
    };
    let cls = vi_class(c0);
    while let Some((i, c)) = cur {
        if vi_class(c) != cls {
            break;
        }
        boundary = i;
        cur = it.next();
    }
    boundary
}

/// vi `e`: byte index of the last character of the current/next small
/// word (the cursor lands *on* it; operators over it are inclusive).
fn vi_word_end(s: &str, pos: usize) -> usize {
    let Some(start) = s[pos..].chars().next().map(|c| pos + c.len_utf8()) else {
        return pos;
    };
    let mut it = s[start..]
        .char_indices()
        .map(|(i, c)| (start + i, c))
        .peekable();
    while it.peek().is_some_and(|&(_, c)| vi_class(c) == 0) {
        it.next();
    }
    let Some(&(first, c0)) = it.peek() else {
        return pos;
    };
    let cls = vi_class(c0);
    let mut last = first;
    while let Some(&(i, c)) = it.peek() {
        if vi_class(c) != cls {
            break;
        }
        last = i;
        it.next();
    }
    last
}

/// vi `%`: the position of the bracket matching the one at (or, like
/// vim, the first one after) the cursor. `None` when there is no bracket
/// or no match.
fn vi_match_bracket(s: &str, pos: usize) -> Option<usize> {
    const OPEN: &str = "([{";
    const CLOSE: &str = ")]}";
    let (at, c) = s[pos..]
        .char_indices()
        .map(|(i, c)| (pos + i, c))
        .find(|&(_, c)| OPEN.contains(c) || CLOSE.contains(c))?;
    let (open, close, forward) = if let Some(k) = OPEN.find(c) {
        (c, CLOSE.chars().nth(k).unwrap(), true)
    } else {
        let k = CLOSE.find(c).unwrap();
        (OPEN.chars().nth(k).unwrap(), c, false)
    };
    let mut depth = 0usize;
    if forward {
        for (i, ch) in s[at..].char_indices() {
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return Some(at + i);
                }
            }
        }
    } else {
        for (i, ch) in s[..at + close.len_utf8()].char_indices().rev() {
            if ch == close {
                depth += 1;
            } else if ch == open {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// The vi `iw`/`aw` text-object span at `pos`: the run of the cursor
/// character's class (`iw`; a whitespace run is its own object, like
/// vim), extended for `aw` over the trailing whitespace — or, when there
/// is none, the leading whitespace instead (vim's rule).
fn vi_word_object(s: &str, pos: usize, around: bool) -> Option<(usize, usize)> {
    if s.is_empty() {
        return None;
    }
    // Clamp a cursor at end-of-line onto the last character, as vi does.
    let pos = if pos >= s.len() {
        s.char_indices().next_back()?.0
    } else {
        pos
    };
    let cls = vi_class(s[pos..].chars().next()?);
    let mut start = pos;
    while let Some((i, c)) = s[..start].char_indices().next_back() {
        if vi_class(c) != cls {
            break;
        }
        start = i;
    }
    let mut end = pos;
    for (i, c) in s[pos..].char_indices() {
        if vi_class(c) != cls {
            end = pos + i;
            break;
        }
        end = pos + i + c.len_utf8();
    }
    if around && cls != 0 {
        let with_trailing = s[end..]
            .find(|c: char| !c.is_whitespace())
            .map(|i| end + i)
            .unwrap_or(s.len());
        if with_trailing > end {
            end = with_trailing;
        } else {
            while let Some((i, c)) = s[..start].char_indices().next_back() {
                if !c.is_whitespace() {
                    break;
                }
                start = i;
            }
        }
    }
    Some((start, end))
}

/// Delete `start..end` into the kill ring. Consecutive kills grow one
/// ring entry (appending for forward kills, prepending for backward) —
/// readline's rule, so C-w C-w C-y restores both words.
fn kill_span(
    st: &mut LineState,
    ring: &mut VecDeque<String>,
    start: usize,
    end: usize,
    forward: bool,
) {
    if start >= end {
        return;
    }
    let text = st.buffer[start..end].to_string();
    let appending = matches!(st.prev_action, Action::KillFwd | Action::KillBack);
    if appending && let Some(last) = ring.back_mut() {
        if forward {
            last.push_str(&text);
        } else {
            last.insert_str(0, &text);
        }
    } else {
        push_ring(ring, text, st.cfg.max_kill_ring);
    }
    st.buffer.replace_range(start..end, "");
    st.cursor = start;
    st.this_action = if forward {
        Action::KillFwd
    } else {
        Action::KillBack
    };
}

fn push_ring(ring: &mut VecDeque<String>, text: String, max: usize) {
    if max == 0 {
        return;
    }
    ring.push_back(text);
    while ring.len() > max {
        ring.pop_front();
    }
}

/// C-y: insert the top of the kill ring at the cursor.
fn yank(st: &mut LineState, ring: &VecDeque<String>) {
    let Some(text) = ring.back() else { return };
    let start = st.cursor;
    st.buffer.insert_str(start, text);
    st.cursor = start + text.len();
    st.yank = Some((start, st.cursor, ring.len() - 1));
    st.this_action = Action::Yank;
}

/// M-y immediately after a yank: rotate the yanked text to the previous
/// ring entry.
fn yank_pop(st: &mut LineState, ring: &VecDeque<String>) {
    if st.prev_action != Action::Yank || ring.is_empty() {
        return;
    }
    let Some((start, end, idx)) = st.yank else {
        return;
    };
    let new_idx = if idx == 0 { ring.len() - 1 } else { idx - 1 };
    let text = &ring[new_idx];
    st.buffer.replace_range(start..end, text);
    st.cursor = start + text.len();
    st.yank = Some((start, st.cursor, new_idx));
    st.this_action = Action::Yank;
}

/// C-_ / C-x C-u / vi `u`: pop the undo stack.
fn undo_cmd(st: &mut LineState) {
    if let Some((buf, cur)) = st.undo.pop_back() {
        st.cursor = cur.min(buf.len());
        st.buffer = buf;
    }
    st.this_action = Action::Undo;
}

/// C-t: transpose the characters around the cursor (at end of line, the
/// two before it — readline's rule).
fn transpose(st: &mut LineState) {
    if st.cursor == st.buffer.len()
        && let Some(prev) = prev_char_start(&st.buffer, st.cursor)
    {
        st.cursor = prev;
    }
    if let Some(prev) = prev_char_start(&st.buffer, st.cursor)
        && let Some(next) = next_char_end(&st.buffer, st.cursor)
    {
        let a: String = st.buffer[prev..st.cursor].to_string();
        let b: String = st.buffer[st.cursor..next].to_string();
        st.buffer.replace_range(prev..next, &format!("{b}{a}"));
        st.cursor = prev + b.len() + a.len();
    }
}

/// M-t: transpose the word at/after the cursor with the one before it,
/// leaving the cursor after the moved pair.
fn transpose_words(st: &mut LineState) {
    // Start of the maximal run of `pred` characters ending at `from`.
    fn run_start(s: &str, from: usize, pred: impl Fn(char) -> bool) -> usize {
        let mut b = from;
        for (i, c) in s[..from].char_indices().rev() {
            if !pred(c) {
                break;
            }
            b = i;
        }
        b
    }
    let e2 = word_forward_alnum(&st.buffer, st.cursor);
    let s2 = run_start(&st.buffer, e2, char::is_alphanumeric);
    let e1 = run_start(&st.buffer, s2, |c| !c.is_alphanumeric());
    let s1 = run_start(&st.buffer, e1, char::is_alphanumeric);
    if s1 >= e1 || s2 >= e2 || e1 > s2 {
        return;
    }
    let w1 = st.buffer[s1..e1].to_string();
    let sep = st.buffer[e1..s2].to_string();
    let w2 = st.buffer[s2..e2].to_string();
    st.buffer.replace_range(s1..e2, &format!("{w2}{sep}{w1}"));
    st.cursor = s1 + w2.len() + sep.len() + w1.len();
}

enum CaseOp {
    Upper,
    Lower,
    Capital,
}

/// M-u / M-l / M-c: upcase / downcase / capitalize from the cursor to the
/// end of the word, moving the cursor past it.
fn case_word(st: &mut LineState, op: CaseOp) {
    let end = word_forward_alnum(&st.buffer, st.cursor);
    if end <= st.cursor {
        return;
    }
    let seg = st.buffer[st.cursor..end].to_string();
    let mut out = String::with_capacity(seg.len());
    let mut first = true;
    for c in seg.chars() {
        match op {
            CaseOp::Upper => out.extend(c.to_uppercase()),
            CaseOp::Lower => out.extend(c.to_lowercase()),
            CaseOp::Capital => {
                if c.is_alphanumeric() && first {
                    first = false;
                    out.extend(c.to_uppercase());
                } else if c.is_alphanumeric() {
                    out.extend(c.to_lowercase());
                } else {
                    out.push(c);
                }
            }
        }
    }
    st.buffer.replace_range(st.cursor..end, &out);
    st.cursor += out.len();
}

/// M-. / M-_: insert the last word of the previous history entry;
/// repeated presses cycle back through older entries, replacing the
/// previous insertion.
fn insert_last_arg(st: &mut LineState, history: &[String]) {
    if history.is_empty() {
        return;
    }
    let (idx, span) = if st.prev_action == Action::LastArg
        && let Some((i, s, e)) = st.lastarg
    {
        (i.saturating_sub(1), Some((s, e)))
    } else {
        (history.len() - 1, None)
    };
    let word = history[idx]
        .split_whitespace()
        .last()
        .unwrap_or("")
        .to_string();
    let start = match span {
        Some((s, e)) => {
            st.buffer.replace_range(s..e, "");
            s
        }
        None => st.cursor,
    };
    st.buffer.insert_str(start, &word);
    st.cursor = start + word.len();
    st.lastarg = Some((idx, start, start + word.len()));
    st.this_action = Action::LastArg;
}

/// C-] (readline `character-search`): read one character and move the
/// cursor to its next occurrence — backward for
/// `character-search-backward`. Reuses the vi `f`/`F` find machinery.
fn character_search(st: &mut LineState, backward: bool) -> io::Result<()> {
    if !wait_for_key(st)? {
        return Ok(()); // deadline: the main loop times out
    }
    let Some(Key::Char(target)) = read_key(st.hooks)? else {
        return Ok(()); // a non-character key aborts, like readline
    };
    let kind = if backward { 'F' } else { 'f' };
    match vi_find_target(&st.buffer, st.cursor, kind, target) {
        Some((t, _)) => st.cursor = t,
        None => bell(st.cfg.bell)?,
    }
    Ok(())
}

/// C-v / C-q: insert the next key literally (a tab, an ESC, a ^C…);
/// the render shows it `^X`-style.
fn quoted_insert(st: &mut LineState) -> io::Result<()> {
    if !wait_for_key(st)? {
        return Ok(()); // deadline: the main loop times out
    }
    let hooks = st.hooks;
    if let Some(b) = read_byte(hooks)? {
        let c = if b < 0x80 {
            b as char
        } else {
            read_utf8(hooks, b)?
        };
        insert_char(st, c);
        st.this_action = Action::Insert;
    }
    Ok(())
}

/// C-x C-e (emacs) / `v` (vi normal): hand the line to `$VISUAL`/`$EDITOR`
/// in a temp file; on a clean exit the edited text is returned and — like
/// bash — executed immediately.
/// Create the C-x C-e scratch file: an unpredictable name claimed with
/// exclusive creation (`O_EXCL`/`CREATE_NEW`, `0600` permissions on Unix —
/// Windows has no mode-bit equivalent, but the exclusive-create check
/// still holds), so a shared `$TMPDIR` gives no symlink-attack window and
/// other users can't read the command line where mode bits apply.
fn create_edit_tempfile(contents: &str) -> io::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir();
    let mut nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0)
        ^ (std::process::id() as u64) << 32;
    for _ in 0..64 {
        // xorshift over the time/pid seed: unpredictable enough for a
        // name that the exclusive-create check then authoritatively backs.
        nonce ^= nonce << 13;
        nonce ^= nonce >> 7;
        nonce ^= nonce << 17;
        let path = dir.join(format!("rusty-lines-edit-{nonce:016x}.txt"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(&path) {
            Ok(mut f) => {
                f.write_all(contents.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique edit tempfile",
    ))
}

fn edit_in_editor(
    st: &mut LineState,
    raw: &RawMode,
    history: &[String],
) -> io::Result<Option<String>> {
    let path = create_edit_tempfile(&st.buffer)?;
    finish_line(st, history)?;
    raw.suspend();
    let editor = st
        .hooks
        .external_editor()
        .or_else(|| std::env::var("VISUAL").ok().filter(|e| !e.is_empty()))
        .or_else(|| std::env::var("EDITOR").ok().filter(|e| !e.is_empty()))
        .unwrap_or_else(|| "vi".to_string());
    // The editor value is a shell command (it may carry arguments, like
    // $EDITOR anywhere); the path is data — single-quote it so a
    // space-bearing $TMPDIR survives.
    let quoted = format!("'{}'", path.display().to_string().replace('\'', r"'\''"));
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} {quoted}"))
        .status();
    raw.resume();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(s) if s.success() => Ok(Some(text.trim_end_matches('\n').to_string())),
        _ => {
            // Editor failed or was declined: repaint on a fresh region.
            st.painted_rows = 1;
            st.painted_cursor_row = 0;
            st.fresh_region = true;
            Ok(None)
        }
    }
}

/// Stash the buffer as an in-session edit of the current history entry
/// (or clear a stale stash when the edit was reverted) before navigating
/// away — so coming back shows the user's version, not the original.
fn stash_hist_edit(st: &mut LineState, history: &[String]) {
    if let Some(i) = st.hist_index {
        if history.get(i) == Some(&st.buffer) {
            st.hist_edits.remove(&i);
        } else {
            st.hist_edits.insert(i, st.buffer.clone());
        }
    }
}

/// The text to show for history entry `i`: the in-session edit if one
/// exists, else the original.
fn hist_entry(st: &LineState, history: &[String], i: usize) -> String {
    st.hist_edits
        .get(&i)
        .cloned()
        .unwrap_or_else(|| history[i].clone())
}

fn history_prev(st: &mut LineState, history: &[String]) {
    stash_hist_edit(st, history);
    let next_index = match st.hist_index {
        None if history.is_empty() => {
            let _ = bell(st.cfg.bell); // nothing to recall
            return;
        }
        None => {
            st.draft = st.buffer.clone();
            history.len() - 1
        }
        Some(0) => {
            let _ = bell(st.cfg.bell); // already at the oldest entry
            return;
        }
        Some(i) => i - 1,
    };
    st.hist_index = Some(next_index);
    st.buffer = hist_entry(st, history, next_index);
    st.cursor = st.buffer.len();
}

fn history_next(st: &mut LineState, history: &[String]) {
    stash_hist_edit(st, history);
    match st.hist_index {
        None => {
            let _ = bell(st.cfg.bell); // already on the live draft
        }
        Some(i) if i + 1 < history.len() => {
            st.hist_index = Some(i + 1);
            st.buffer = hist_entry(st, history, i + 1);
            st.cursor = st.buffer.len();
        }
        Some(_) => {
            st.hist_index = None;
            st.buffer = std::mem::take(&mut st.draft);
            st.cursor = st.buffer.len();
        }
    }
}

/// M-<: jump to the oldest history entry.
fn history_first(st: &mut LineState, history: &[String]) {
    if history.is_empty() {
        return;
    }
    stash_hist_edit(st, history);
    if st.hist_index.is_none() {
        st.draft = st.buffer.clone();
    }
    st.hist_index = Some(0);
    st.buffer = hist_entry(st, history, 0);
    st.cursor = st.buffer.len();
}

/// M->: back to the live (draft) line.
fn history_last(st: &mut LineState, history: &[String]) {
    stash_hist_edit(st, history);
    if st.hist_index.is_some() {
        st.hist_index = None;
        st.buffer = std::mem::take(&mut st.draft);
        st.cursor = st.buffer.len();
    }
}

/// PageUp / M-p: previous history entry starting with the text before the
/// cursor (zsh's `history-beginning-search-backward`, fish's Up).
fn history_prefix_prev(st: &mut LineState, history: &[String]) {
    // Re-anchor on the buffer up to the cursor whenever the previous key
    // wasn't itself a prefix search (zsh's rule) — anchoring only when
    // off history left a stale/empty prefix after a plain Up.
    if st.prev_action != Action::PrefixSearch {
        st.prefix = st.buffer[..st.cursor].to_string();
    }
    st.this_action = Action::PrefixSearch;
    stash_hist_edit(st, history);
    if st.hist_index.is_none() {
        st.draft = st.buffer.clone();
    }
    let below = st.hist_index.unwrap_or(history.len());
    let ci = st.cfg.search_ignore_case;
    if let Some(i) = history[..below.min(history.len())]
        .iter()
        .rposition(|h| starts_with_match(h, &st.prefix, ci) && *h != st.buffer)
    {
        st.hist_index = Some(i);
        st.buffer = hist_entry(st, history, i);
        st.cursor = st.buffer.len();
    } else {
        let _ = bell(st.cfg.bell); // no earlier entry with this prefix
    }
}

/// PageDown / M-n: next prefix match, or back to the draft.
fn history_prefix_next(st: &mut LineState, history: &[String]) {
    st.this_action = Action::PrefixSearch;
    stash_hist_edit(st, history);
    let Some(cur) = st.hist_index else { return };
    let ci = st.cfg.search_ignore_case;
    if let Some(off) = history[cur + 1..]
        .iter()
        .position(|h| starts_with_match(h, &st.prefix, ci))
    {
        let i = cur + 1 + off;
        st.hist_index = Some(i);
        st.buffer = hist_entry(st, history, i);
        st.cursor = st.buffer.len();
    } else {
        st.hist_index = None;
        st.buffer = std::mem::take(&mut st.draft);
        st.cursor = st.buffer.len();
    }
}

/// Tab completion: insert the longest
/// common prefix; when that makes no progress, print the columned
/// candidate list below the line and arm menu cycling, so further Tabs
/// walk the candidates in-line (zsh `AUTO_MENU`).
fn complete_at_cursor(st: &mut LineState, history: &[String]) -> io::Result<()> {
    let (start, mut candidates) = st.hooks.complete(&st.buffer, st.cursor);
    let start = clamp_start(&st.buffer, start, st.cursor);
    if candidates.is_empty() {
        return bell(st.cfg.bell);
    }
    // readline sorts its matches; hosts get the same order in the list
    // and the menu cycle.
    candidates.sort_by(|a, b| a.display.cmp(&b.display));
    let lcp = common_prefix(
        &candidates
            .iter()
            .map(|c| c.replacement.as_str())
            .collect::<Vec<_>>(),
        st.cfg.completion_ignore_case,
    );
    let current = &st.buffer[start..st.cursor];
    let progressed = lcp.len() > current.len();
    if progressed {
        st.buffer.replace_range(start..st.cursor, &lcp);
        st.cursor = start + lcp.len();
    }
    if candidates.len() == 1 {
        // A unique match, now fully inserted: append the completion
        // append character (readline's, a space by default) so the
        // cursor is ready for the next word.
        if let Some(c) = st.cfg.completion_append {
            st.buffer.insert(st.cursor, c);
            st.cursor += c.len_utf8();
        }
        return Ok(());
    }
    // show-all-if-ambiguous (readline): list right away instead of
    // waiting for a second Tab after the prefix insertion.
    if progressed && !st.cfg.show_all_if_ambiguous {
        return Ok(());
    }
    if list_candidates(st, &candidates, history)? {
        st.menu = Some(MenuState {
            start,
            inserted: st.cursor - start,
            index: None,
            candidates,
        });
    }
    Ok(())
}

/// Leave the edit region and print the candidate list (behind the
/// `completion-query-items` question when it's big). `false` when the
/// user declined or the deadline passed — nothing was listed.
fn list_candidates(
    st: &mut LineState,
    candidates: &[Candidate],
    history: &[String],
) -> io::Result<bool> {
    // The next render starts a fresh region below whatever gets printed.
    finish_line(st, history)?;
    st.painted_rows = 1;
    st.painted_cursor_row = 0;
    st.fresh_region = true;
    // readline's completion-query-items: a big list asks first.
    if st.cfg.completion_query_items > 0 && candidates.len() >= st.cfg.completion_query_items {
        write!(
            io::stdout(),
            "Display all {} possibilities? (y or n)",
            candidates.len()
        )?;
        io::stdout().flush()?;
        if !wait_for_key(st)? {
            writeln!(io::stdout())?;
            return Ok(false); // deadline: the main loop times out
        }
        let yes = matches!(
            read_key(st.hooks)?,
            Some(Key::Char('y' | 'Y' | ' ')) | Some(Key::Tab)
        );
        writeln!(io::stdout())?;
        if !yes {
            return Ok(false);
        }
    }
    print_candidate_columns(candidates)?;
    Ok(true)
}

/// Print the candidate list in vertical (column-major) order, the way
/// readline lays out its completion listing.
fn print_candidate_columns(candidates: &[Candidate]) -> io::Result<()> {
    let width = candidates
        .iter()
        .map(|c| display_width(&c.display))
        .max()
        .unwrap_or(0)
        + 2;
    let cols = (term_cols() / width.max(1)).max(1);
    let rows = candidates.len().div_ceil(cols);
    for r in 0..rows {
        let mut line = String::new();
        for c in 0..cols {
            if let Some(cand) = candidates.get(c * rows + r) {
                line.push_str(&cand.display);
                let pad = width.saturating_sub(display_width(&cand.display));
                line.extend(std::iter::repeat_n(' ', pad));
            }
        }
        writeln!(io::stdout(), "{}", line.trim_end())?;
    }
    Ok(())
}

/// readline `menu-complete` (also Tab under `set_menu_complete`): insert
/// the first candidate immediately and arm cycling — no LCP step, no
/// candidate list.
fn menu_complete_start(st: &mut LineState) -> io::Result<()> {
    if menu_arm(st)? {
        menu_next(st);
    }
    Ok(())
}

/// Ask the host for candidates and arm the cycling state without
/// stepping. `false` (after ringing the bell) when there are none.
fn menu_arm(st: &mut LineState) -> io::Result<bool> {
    let (start, mut candidates) = st.hooks.complete(&st.buffer, st.cursor);
    let start = clamp_start(&st.buffer, start, st.cursor);
    if candidates.is_empty() {
        bell(st.cfg.bell)?;
        return Ok(false);
    }
    candidates.sort_by(|a, b| a.display.cmp(&b.display));
    st.menu = Some(MenuState {
        start,
        inserted: st.cursor - start,
        index: None,
        candidates,
    });
    Ok(true)
}

/// A further Tab with the menu armed: replace the word with the next
/// candidate, wrapping around the list.
fn menu_next(st: &mut LineState) {
    menu_step(st, false)
}

/// One step of the cycle, wrapping in either direction; a cold backward
/// step starts on the last candidate.
fn menu_step(st: &mut LineState, backward: bool) {
    let Some(mut menu) = st.menu.take() else {
        return;
    };
    let len = menu.candidates.len();
    let i = match (menu.index, backward) {
        (None, false) => 0,
        (None, true) => len - 1,
        (Some(i), false) => (i + 1) % len,
        (Some(i), true) => (i + len - 1) % len,
    };
    menu.index = Some(i);
    let replacement = &menu.candidates[i].replacement;
    st.buffer
        .replace_range(menu.start..menu.start + menu.inserted, replacement);
    st.cursor = menu.start + replacement.len();
    menu.inserted = replacement.len();
    st.menu = Some(menu);
}

fn longest_common_prefix(names: &[&str]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut prefix = first.to_string();
    for name in &names[1..] {
        while !name.starts_with(&prefix) {
            prefix.pop();
            if prefix.is_empty() {
                return prefix;
            }
        }
    }
    prefix
}

/// `longest_common_prefix`, optionally case-insensitive (readline's
/// `completion-ignore-case`): candidates are compared ignoring case and
/// the first candidate's spelling is what gets inserted.
fn common_prefix(names: &[&str], ignore_case: bool) -> String {
    if !ignore_case {
        return longest_common_prefix(names);
    }
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut end = first.len();
    for name in &names[1..] {
        let mut common = 0;
        for (a, b) in first[..end].chars().zip(name.chars()) {
            if a != b && !a.to_lowercase().eq(b.to_lowercase()) {
                break;
            }
            common += a.len_utf8();
        }
        end = common;
        if end == 0 {
            break;
        }
    }
    first[..end].to_string()
}

/// The host's hint for the current buffer, memoized on the buffer
/// content so each keystroke asks [`Hooks::hint`] at most once — the
/// render and the accept paths (Right/End/M-f) all consult it, and a
/// hint hook doing history scans or filesystem checks is the expensive
/// kind (fish caches its autosuggestion the same way).
fn cached_hint(st: &mut LineState, history: &[String]) -> Option<String> {
    if let Some((buf, hint)) = &st.hint_cache
        && *buf == st.buffer
    {
        return hint.clone();
    }
    let hint = st.hooks.hint(&st.buffer, history);
    st.hint_cache = Some((st.buffer.clone(), hint.clone()));
    hint
}

/// Repaint the whole edit region and reposition the cursor.
///
/// Layout math: everything is measured in display columns (ANSI-skipped,
/// wide-character-aware, control characters via `visualize`). When a
/// painted row ends exactly at the terminal width, a newline is emitted to
/// force the wrap immediately — sidestepping terminals' delayed-wrap
/// state, which would otherwise break the relative cursor movements the
/// next repaint starts with.
fn render(st: &mut LineState, history: &[String]) -> io::Result<()> {
    let cols = term_cols().max(2);
    let mut out = String::new();

    // Return to the region's first row/column. The old content is *not*
    // cleared here: the new paint overwrites it in place and a trailing
    // `ESC[J` erases whatever it left over — clear-then-paint showed a
    // blank frame every keystroke (visible flicker on slow terminals).
    out.push('\r');
    if st.painted_cursor_row > 0 {
        out.push_str(&format!("\x1b[{}A", st.painted_cursor_row));
    }

    // Search mode paints its own prompt instead of PS1/buffer.
    if let Some(search) = &st.search {
        let shown = search.hit.map(|i| history[i].as_str()).unwrap_or("");
        let label = match (search.failed, search.forward) {
            (false, false) => "(reverse-i-search)",
            (false, true) => "(i-search)",
            (true, false) => "(failed reverse-i-search)",
            (true, true) => "(failed i-search)",
        };
        let head = format!("{label}`{}': ", search.query);
        let line = format!("{}{}", head, visualize(shown, display_width(&head)));
        out.push_str(&line);
        out.push_str("\x1b[J");
        let w = display_width(&line);
        st.painted_rows = w / cols + 1;
        st.painted_cursor_row = w / cols;
        let mut stdout = io::stdout().lock();
        stdout.write_all(out.as_bytes())?;
        return stdout.flush();
    }

    // A multi-line prompt: everything up to the last newline is printed
    // once per region (readline's approach) — only the final line is
    // part of the per-keystroke repaint.
    let (prompt_prefix, prompt_line) = match st.prompt.rsplit_once('\n') {
        Some((prefix, last)) => (Some(prefix), last),
        None => (None, st.prompt),
    };
    if st.fresh_region {
        st.fresh_region = false;
        if let Some(prefix) = prompt_prefix {
            out.push_str(prefix);
            out.push('\n');
        }
    }

    // readline's show-mode-in-prompt, with its default mode strings
    // (prefixed to the prompt's final line).
    let mode = if !st.cfg.show_mode_in_prompt {
        ""
    } else if !st.vi {
        "@"
    } else if st.vi_normal {
        "(cmd)"
    } else {
        "(ins)"
    };
    // Buffer visualization starts at the prompt's end column: tab stops
    // depend on it.
    let wp = display_width(mode) + display_width(prompt_line);

    // One pass over the buffer measures the paint, the cursor column,
    // and the total width together.
    let (vis, wcursor, vis_end) = visualize_marked(&st.buffer, wp, st.cursor);
    let wb = vis_end - wp;
    // The host highlights the raw buffer (true text, true offsets); its
    // SGR markup survives and everything else is re-visualized. A buffer
    // holding a literal ESC is painted unhighlighted — hook markup and
    // buffer bytes would be indistinguishable.
    let highlighted = if st.buffer.contains('\x1b') {
        vis.clone()
    } else {
        visualize_keep_sgr(&st.hooks.highlight(&st.buffer), wp)
    };
    let hint = if st.cursor == st.buffer.len() {
        visualize(&cached_hint(st, history).unwrap_or_default(), wp + wb)
    } else {
        String::new()
    };
    let wh = display_width(&hint);
    let wtotal = wp + wb + wh;

    out.push_str(mode);
    out.push_str(prompt_line);
    out.push_str(&highlighted);
    if !hint.is_empty() {
        out.push_str(&format!("\x1b[2m{hint}\x1b[0m"));
    }

    // The right prompt: shown while everything fits on one row with
    // a gap; hidden (zsh-style) once the line grows into it. Clear the
    // gap first — the new content may be shorter than the old.
    let wr = display_width(st.rprompt);
    if wr > 0 && wtotal + wr + 1 < cols {
        out.push_str("\x1b[K");
        out.push_str(&format!("\x1b[{}G", cols - wr + 1));
        out.push_str(st.rprompt);
    }

    // Force the wrap when the content ends exactly on the boundary —
    // after which the cursor sits at (wtotal / cols, 0) either way.
    if wtotal > 0 && wtotal.is_multiple_of(cols) {
        out.push_str("\r\n");
    }

    // Erase whatever the previous (possibly longer) paint left behind:
    // the rest of this row and every row below the new content.
    out.push_str("\x1b[J");

    let total_rows = wtotal / cols + 1;
    let end_row = wtotal / cols;
    let cursor_row = wcursor / cols;
    let cursor_col = wcursor % cols;

    // Reposition from the end of the paint to the cursor.
    let up = end_row.saturating_sub(cursor_row);
    if up > 0 {
        out.push_str(&format!("\x1b[{up}A"));
    }
    out.push_str(&format!("\r\x1b[{}G", cursor_col + 1));

    st.painted_rows = total_rows;
    st.painted_cursor_row = cursor_row;

    let mut stdout = io::stdout().lock();
    stdout.write_all(out.as_bytes())?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(buf: &str, cursor: usize) -> LineState<'static> {
        state_hooked(buf, cursor, &NoHooks)
    }

    fn state_hooked<'a>(buf: &str, cursor: usize, hooks: &'a dyn Hooks) -> LineState<'a> {
        LineState {
            buffer: buf.to_string(),
            cursor,
            prompt: "",
            rprompt: "",
            painted_rows: 1,
            painted_cursor_row: 0,
            fresh_region: true,
            render_owed: false,
            coalesced_run: 0,
            hist_index: None,
            draft: String::new(),
            prefix: String::new(),
            hist_edits: std::collections::HashMap::new(),
            vi: false,
            vi_normal: false,
            vi_count: 0,
            vi_op: None,
            vi_find: None,
            vi_replace: false,
            vi_textobj: None,
            last_find: None,
            search: None,
            undo: VecDeque::new(),
            prev_action: Action::Other,
            this_action: Action::Other,
            edit_run: 0,
            yank: None,
            lastarg: None,
            menu: None,
            deadline: None,
            hint_cache: None,
            cfg: EditorConfig::default(),
            hooks,
        }
    }

    #[test]
    fn width_skips_ansi_and_counts_wide_chars() {
        assert_eq!(display_width("plain"), 5);
        assert_eq!(display_width("\x1b[32mgreen\x1b[0m"), 5);
        assert_eq!(display_width("日本"), 4); // two double-width chars
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn common_prefix_plain() {
        assert_eq!(longest_common_prefix(&["echo", "ech", "echelon"]), "ech");
        assert_eq!(longest_common_prefix(&["abc"]), "abc");
        assert_eq!(longest_common_prefix(&["x", "y"]), "");
        assert_eq!(longest_common_prefix(&[]), "");
    }

    #[test]
    fn word_motion() {
        assert_eq!(word_back("echo hello", 10), 5);
        assert_eq!(word_back("echo hello", 5), 0);
        assert_eq!(word_back("word", 4), 0);
    }

    #[test]
    fn word_motion_alnum() {
        // M-b/M-f stop at non-alphanumerics, unlike C-w's whitespace rule.
        assert_eq!(word_back_alnum("a-b/c.txt", 9), 6); // back over "txt"
        assert_eq!(word_back_alnum("echo hello", 10), 5);
        assert_eq!(word_back_alnum("  ", 2), 0); // no word: to line start
        assert_eq!(word_forward_alnum("a-b/c.txt", 0), 1); // just "a"
        assert_eq!(word_forward_alnum("--foo bar", 0), 5); // skips the dashes
    }

    #[test]
    fn vi_small_words() {
        //           0123456789
        let s = "ab cd-ef gh";
        assert_eq!(vi_word_fwd(s, 0), 3); // ab → cd
        assert_eq!(vi_word_fwd(s, 3), 5); // cd → the "-" (its own word)
        assert_eq!(vi_word_fwd(s, 5), 6); // "-" → ef
        assert_eq!(vi_word_back(s, 6), 5); // ef → "-"
        assert_eq!(vi_word_back(s, 3), 0);
        assert_eq!(vi_word_end(s, 0), 1); // on the "b" of "ab"
        assert_eq!(vi_word_end(s, 1), 4); // b → d of "cd"
    }

    #[test]
    fn vi_find_targets() {
        //       0123456
        let s = "echo ab";
        assert_eq!(vi_find_target(s, 0, 'f', 'o'), Some((3, true)));
        assert_eq!(vi_find_target(s, 0, 't', 'o'), Some((2, true)));
        assert_eq!(vi_find_target(s, 6, 'F', 'e'), Some((0, false)));
        assert_eq!(vi_find_target(s, 6, 'T', 'e'), Some((1, false)));
        assert_eq!(vi_find_target(s, 0, 'f', 'z'), None);
    }

    #[test]
    fn visualize_control_chars() {
        {
            assert_eq!(visualize("plain", 0), "plain");
            assert_eq!(visualize("a\x1bb", 0), "a^[b");
            // Tabs advance to the next 8-column stop of the running
            // display offset, prompt included — not a fixed four spaces.
            assert_eq!(visualize("a\tb", 0), "a       b"); // col 1 → pad 7
            assert_eq!(visualize("\tb", 0), "        b"); // col 0 → pad 8
            assert_eq!(visualize("a\tb", 6), "a b"); // col 7 → pad 1
            assert_eq!(visualize("日\tb", 0), "日      b"); // wide char: col 2
            assert_eq!(visualize("a\nb", 0), "a⏎b");
            assert_eq!(visualize("\u{7f}", 0), "^?");
        }
    }

    #[test]
    fn csi_sequences() {
        assert_eq!(csi_key("", b'A'), Key::Up);
        assert_eq!(csi_key("", b'D'), Key::Left);
        assert_eq!(csi_key("3", b'~'), Key::Delete);
        assert_eq!(csi_key("1", b'~'), Key::Home);
        assert_eq!(csi_key("1;5", b'C'), Key::WordRight);
        assert_eq!(csi_key("1;3", b'D'), Key::WordLeft);
        assert_eq!(csi_key("5", b'~'), Key::PageUp);
        assert_eq!(csi_key("6", b'~'), Key::PageDown);
        assert_eq!(csi_key("99", b'~'), Key::Other);
    }

    #[test]
    fn history_dedups_consecutive_only() {
        let mut ed = Editor::new();
        ed.add_history_entry("a");
        ed.add_history_entry("a");
        ed.add_history_entry("b");
        ed.add_history_entry("a");
        assert_eq!(ed.history(), &["a", "b", "a"]);
    }

    #[test]
    fn history_joins_multiline_entries() {
        let mut ed = Editor::new();
        ed.add_history_entry("echo a\necho b");
        assert_eq!(ed.history(), &["echo a; echo b"]);
    }

    #[test]
    fn kill_ring_appends_consecutive_kills() {
        let mut ring = VecDeque::new();
        let mut st = state("one two three", 13);
        // C-w twice: "three" then "two " prepends onto the same entry.
        let (start, cur) = (word_back(&st.buffer, st.cursor), st.cursor);
        kill_span(&mut st, &mut ring, start, cur, false);
        st.prev_action = st.this_action;
        let (start, cur) = (word_back(&st.buffer, st.cursor), st.cursor);
        kill_span(&mut st, &mut ring, start, cur, false);
        assert_eq!(st.buffer, "one ");
        assert_eq!(ring, vec!["two three".to_string()]);
        // A yank restores both words at once.
        st.prev_action = Action::Other;
        yank(&mut st, &ring);
        assert_eq!(st.buffer, "one two three");
    }

    #[test]
    fn yank_pop_rotates_ring() {
        let mut ring = VecDeque::from(vec!["old".to_string(), "new".to_string()]);
        let mut st = state("", 0);
        yank(&mut st, &ring);
        assert_eq!(st.buffer, "new");
        st.prev_action = st.this_action;
        yank_pop(&mut st, &ring);
        assert_eq!(st.buffer, "old");
        st.prev_action = st.this_action;
        yank_pop(&mut st, &ring);
        assert_eq!(st.buffer, "new");
        let _ = &mut ring;
    }

    #[test]
    fn transpose_words_swaps_around_cursor() {
        let mut st = state("echo one two", 12);
        transpose_words(&mut st);
        assert_eq!(st.buffer, "echo two one");
        assert_eq!(st.cursor, 12);
        // A single word: no-op.
        let mut st = state("word", 4);
        transpose_words(&mut st);
        assert_eq!(st.buffer, "word");
    }

    #[test]
    fn case_ops() {
        let mut st = state("echo word here", 5);
        case_word(&mut st, CaseOp::Upper);
        assert_eq!(st.buffer, "echo WORD here");
        assert_eq!(st.cursor, 9);
        let mut st = state("echo WORD", 5);
        case_word(&mut st, CaseOp::Lower);
        assert_eq!(st.buffer, "echo word");
        let mut st = state("echo wOrD", 5);
        case_word(&mut st, CaseOp::Capital);
        assert_eq!(st.buffer, "echo Word");
    }

    #[test]
    fn last_arg_inserts_and_cycles() {
        let history = vec!["echo first one".to_string(), "echo second two".to_string()];
        let mut st = state("echo ", 5);
        insert_last_arg(&mut st, &history);
        assert_eq!(st.buffer, "echo two");
        st.prev_action = st.this_action;
        insert_last_arg(&mut st, &history);
        assert_eq!(st.buffer, "echo one");
    }

    #[test]
    fn undo_restores_snapshots() {
        let mut st = state("hello", 5);
        st.undo.push_back(("hell".to_string(), 4));
        st.undo.push_back(("hello".to_string(), 5));
        undo_cmd(&mut st);
        assert_eq!(st.buffer, "hello");
        undo_cmd(&mut st);
        assert_eq!(st.buffer, "hell");
        assert_eq!(st.cursor, 4);
        undo_cmd(&mut st); // empty stack: no-op
        assert_eq!(st.buffer, "hell");
    }

    #[test]
    fn prefix_search_matches_start_only() {
        let history = vec![
            "git status".to_string(),
            "echo git".to_string(),
            "git push".to_string(),
        ];
        let mut st = state("git", 3);
        // Consecutive presses keep the anchor (the main loop's
        // prev_action bookkeeping, mirrored here between calls).
        history_prefix_prev(&mut st, &history);
        st.prev_action = st.this_action;
        assert_eq!(st.buffer, "git push");
        history_prefix_prev(&mut st, &history);
        st.prev_action = st.this_action;
        assert_eq!(st.buffer, "git status"); // skipped "echo git"
        history_prefix_next(&mut st, &history);
        st.prev_action = st.this_action;
        assert_eq!(st.buffer, "git push");
        history_prefix_next(&mut st, &history);
        st.prev_action = st.this_action;
        assert_eq!(st.buffer, "git"); // back to the draft
    }

    #[test]
    fn prefix_search_reanchors_after_other_keys() {
        let history = vec!["echo x".to_string(), "git status".to_string()];
        // Plain Up recalls "git status"; a following PageUp must anchor
        // on the *current* buffer up to the cursor — the old behavior
        // kept a stale empty prefix and walked to "echo x".
        let mut st = state("git", 3);
        st.cfg.bell = BellStyle::None;
        history_prev(&mut st, &history);
        st.prev_action = st.this_action; // Action::Other
        assert_eq!(st.buffer, "git status");
        history_prefix_prev(&mut st, &history);
        assert_eq!(st.buffer, "git status", "no earlier 'git status…' entry");
        assert_eq!(st.prefix, "git status");
    }

    #[test]
    fn history_cap_drops_oldest() {
        let mut ed = Editor::new();
        ed.set_max_history_len(2);
        ed.add_history_entry("a");
        ed.add_history_entry("b");
        ed.add_history_entry("c");
        assert_eq!(ed.history(), ["b", "c"]);
        ed.set_max_history_len(1); // shrinking trims immediately
        assert_eq!(ed.history(), ["c"]);
    }

    #[test]
    fn append_history_writes_only_new_entries() {
        let path =
            std::env::temp_dir().join(format!("rusty_lines_hist_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut ed = Editor::new();
        ed.add_history_entry("one");
        ed.save_history(&path).unwrap();
        ed.add_history_entry("two");
        ed.append_history(&path).unwrap();
        ed.append_history(&path).unwrap(); // nothing new: no-op
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");

        // A second session appends without clobbering the first's entries.
        let mut ed2 = Editor::new();
        ed2.load_history(&path).unwrap();
        ed2.add_history_entry("three");
        ed2.append_history(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\nthree\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn key_specs_parse_readline_spellings() {
        assert_eq!(parse_key_spec("\\C-f"), Some(Key::Ctrl('f')));
        assert_eq!(parse_key_spec("\\C-F"), Some(Key::Ctrl('f')));
        assert_eq!(parse_key_spec("\\C-?"), Some(Key::Backspace));
        assert_eq!(parse_key_spec("\\C-_"), Some(Key::Ctrl('_')));
        assert_eq!(parse_key_spec("\\M-f"), Some(Key::Alt('f')));
        assert_eq!(parse_key_spec("\\ex"), Some(Key::Alt('x')));
        assert_eq!(parse_key_spec("\\M-\\C-?"), Some(Key::AltBackspace));
        assert_eq!(parse_key_spec("\\e[A"), Some(Key::Up));
        assert_eq!(parse_key_spec("\\e[1;5C"), Some(Key::WordRight));
        assert_eq!(parse_key_spec("\\e[3~"), Some(Key::Delete));
        assert_eq!(parse_key_spec("\\eOH"), Some(Key::Home));
        assert_eq!(parse_key_spec("\\e"), Some(Key::Esc));
        assert_eq!(parse_key_spec("\\C-m"), Some(Key::Enter));
        assert_eq!(parse_key_spec("\\t"), Some(Key::Tab));
        assert_eq!(parse_key_spec("\\x09"), Some(Key::Tab));
        assert_eq!(parse_key_spec("\\011"), Some(Key::Tab));
        assert_eq!(parse_key_spec("a"), Some(Key::Char('a')));
        assert_eq!(parse_key_spec("ü"), Some(Key::Char('ü')));
        // Unparseable: empty, chords, unknown sequences, junk escapes.
        assert_eq!(parse_key_spec(""), None);
        assert_eq!(parse_key_spec("\\C-x\\C-e"), None);
        assert_eq!(parse_key_spec("ab"), None);
        assert_eq!(parse_key_spec("\\e[99~"), None);
        assert_eq!(parse_key_spec("\\Z"), None);
    }

    #[test]
    fn key_specs_round_trip_through_bindings_listing() {
        for (key, _) in DEFAULT_BINDINGS {
            assert_eq!(
                parse_key_spec(&key_spec(key)).as_ref(),
                Some(key),
                "spec {:?} for {key:?} does not round-trip",
                key_spec(key)
            );
        }
    }

    #[test]
    fn bind_overrides_default_and_unbind_masks_it() {
        let mut ed = Editor::new();
        let lookup = |ed: &Editor, spec: &str| -> Option<EditorAction> {
            ed.bindings().find(|(k, _)| k == spec).map(|(_, a)| a)
        };
        assert_eq!(lookup(&ed, "\\C-f"), Some(EditorAction::ForwardChar));

        ed.bind("\\C-f", EditorAction::KillLine).unwrap();
        assert_eq!(lookup(&ed, "\\C-f"), Some(EditorAction::KillLine));
        // The default row is replaced, not duplicated.
        assert_eq!(ed.bindings().filter(|(k, _)| k == "\\C-f").count(), 1);

        ed.unbind("\\C-f").unwrap();
        assert_eq!(lookup(&ed, "\\C-f"), None);

        // Host bindings are stored but not listed as actions.
        ed.bind_host("\\C-g", "fzf".to_string()).unwrap();
        assert_eq!(lookup(&ed, "\\C-g"), None);
        assert!(
            ed.bindings
                .iter()
                .any(|(k, b)| *k == Key::Ctrl('g') && *b == Binding::Host("fzf".to_string()))
        );

        // A bad spec is an InvalidInput error.
        let err = ed.bind("\\C-x\\C-e", EditorAction::Undo).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn rebound_key_runs_the_new_action() {
        // C-f rebound to kill-line: dispatch consults the custom table.
        let mut ed = Editor::new();
        ed.bind("\\C-f", EditorAction::KillLine).unwrap();
        let (key, binding) = &ed.bindings[0];
        assert_eq!(*key, Key::Ctrl('f'));
        let Binding::Action(action) = binding else {
            panic!("expected an action binding");
        };
        let mut st = state("hello", 2);
        let mut ring = VecDeque::new();
        run_action(&mut st, *action, key, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "he");
        assert_eq!(ring, ["llo"]);
    }

    #[test]
    fn menu_complete_inserts_first_candidate_immediately() {
        struct H;
        impl Hooks for H {
            fn complete(&self, line: &str, pos: usize) -> (usize, Vec<Candidate>) {
                let cand = |s: &str| Candidate {
                    display: s.to_string(),
                    replacement: s.to_string(),
                };
                let _ = (line, pos);
                (4, vec![cand("alpha"), cand("alphabet")])
            }
        }
        let mut st = state_hooked("say al", 6, &H);
        st.cfg.menu_complete = true;
        let mut ring = VecDeque::new();
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "say alpha");
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "say alphabet");
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "say alpha"); // wraps
    }

    #[test]
    fn case_insensitive_common_prefix() {
        assert_eq!(common_prefix(&["Echo", "echelon"], true), "Ech");
        assert_eq!(common_prefix(&["Echo", "echelon"], false), "");
        assert_eq!(common_prefix(&["abc", "ABC"], true), "abc");
        assert_eq!(common_prefix(&["x", "y"], true), "");
        assert_eq!(common_prefix(&[], true), "");
    }

    #[test]
    fn history_timestamps_round_trip_both_formats() {
        let path = std::env::temp_dir().join(format!("rusty_lines_ts_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // Adding stamps entries; plain save leaves the file un-timestamped.
        let mut ed = Editor::new();
        ed.add_history_entry("one");
        assert!(ed.history_timestamps()[0].is_some());
        ed.save_history(&path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().starts_with('#'));

        // Timestamped save writes bash's `#<epoch>` comment lines...
        ed.set_history_timestamps(true);
        ed.save_history(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with('#'), "no timestamp line:\n{text}");
        assert!(text.ends_with("one\n"));

        // ...which load back as timestamps, not entries, and append keeps
        // stamping.
        let mut ed2 = Editor::new();
        ed2.set_history_timestamps(true);
        ed2.load_history(&path).unwrap();
        assert_eq!(ed2.history(), ["one"]);
        assert_eq!(ed2.history_timestamps(), ed.history_timestamps());
        ed2.add_history_entry("two");
        ed2.append_history(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text.lines().count(),
            4,
            "expected 2 stamped entries:\n{text}"
        );

        // A plain (never-stamped) file loads with `None` timestamps.
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let mut ed3 = Editor::new();
        ed3.load_history(&path).unwrap();
        assert_eq!(ed3.history(), ["alpha", "beta"]);
        assert_eq!(ed3.history_timestamps(), [None, None]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn replace_history_swaps_entries_and_keeps_appends_incremental() {
        let path =
            std::env::temp_dir().join(format!("rusty_lines_replace_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut ed = Editor::new();
        ed.add_history_entry("a");
        ed.add_history_entry("b");
        ed.replace_history(vec!["b".to_string()]); // history -d dropped "a"
        assert_eq!(ed.history(), ["b"]);
        assert_eq!(ed.history_timestamps(), [None]);
        // Replaced entries count as persisted: only later additions append.
        ed.add_history_entry("c");
        ed.append_history(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "c\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn history_dedup_erases_earlier_duplicates() {
        let mut ed = Editor::new();
        ed.set_history_dedup(true);
        ed.add_history_entry("a");
        ed.add_history_entry("b");
        ed.add_history_entry("a");
        assert_eq!(ed.history(), ["b", "a"]);
        // Off by default: only consecutive repeats are skipped.
        let mut ed = Editor::new();
        ed.add_history_entry("a");
        ed.add_history_entry("b");
        ed.add_history_entry("a");
        assert_eq!(ed.history(), ["a", "b", "a"]);
    }

    #[test]
    fn revert_line_undoes_everything_at_once() {
        let mut st = state("hello", 5);
        st.undo = VecDeque::from(vec![("hello".to_string(), 5), ("hellox".to_string(), 6)]);
        st.buffer = "helloxyz".to_string();
        st.cursor = 8;
        let mut ring = VecDeque::new();
        run_action(
            &mut st,
            EditorAction::RevertLine,
            &Key::Alt('r'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "hello");
        assert_eq!(st.cursor, 5);
        assert!(st.undo.is_empty());
    }

    #[test]
    fn menu_cycling_walks_candidates_and_wraps() {
        let cand = |s: &str| Candidate {
            display: s.to_string(),
            replacement: s.to_string(),
        };
        let mut st = state("say al", 6);
        st.menu = Some(MenuState {
            start: 4,
            inserted: 2,
            index: None,
            candidates: vec![cand("alpha"), cand("alphabet")],
        });
        menu_next(&mut st);
        assert_eq!(st.buffer, "say alpha");
        assert_eq!(st.cursor, 9);
        menu_next(&mut st);
        assert_eq!(st.buffer, "say alphabet");
        menu_next(&mut st); // wraps back around
        assert_eq!(st.buffer, "say alpha");
    }

    #[test]
    fn c0_bytes_outside_ctrl_letters_decode_as_ctrl_keys() {
        // Ctrl-Space / Ctrl-\ / Ctrl-] / Ctrl-^ must not fall through to
        // the character path and self-insert raw control bytes.
        assert_eq!(parse_key_spec("\\C-@"), Some(Key::Ctrl('@')));
        assert_eq!(parse_key_spec("\\C-\\\\"), Some(Key::Ctrl('\\')));
        assert_eq!(parse_key_spec("\\C-]"), Some(Key::Ctrl(']')));
        assert_eq!(parse_key_spec("\\C-^"), Some(Key::Ctrl('^')));
        assert_eq!(decode_key_bytes(&[0x00]), Some(Key::Ctrl('@')));
        assert_eq!(decode_key_bytes(&[0x1c]), Some(Key::Ctrl('\\')));
        // And they round-trip through the spec printer.
        for key in [Key::Ctrl('@'), Key::Ctrl('\\'), Key::Alt('\\')] {
            assert_eq!(parse_key_spec(&key_spec(&key)).as_ref(), Some(&key));
        }
    }

    #[test]
    fn vi_match_bracket_finds_pairs() {
        //       0123456789
        let s = "a(b[c]d)e";
        assert_eq!(vi_match_bracket(s, 1), Some(7)); // ( → )
        assert_eq!(vi_match_bracket(s, 7), Some(1)); // ) → (
        assert_eq!(vi_match_bracket(s, 3), Some(5)); // [ → ]
        assert_eq!(vi_match_bracket(s, 0), Some(7)); // scans forward to (
        assert_eq!(vi_match_bracket("plain", 0), None);
        assert_eq!(vi_match_bracket("(open", 0), None); // unbalanced
    }

    #[test]
    fn vi_word_objects() {
        //       0123456789012345
        let s = "one  two.three";
        assert_eq!(vi_word_object(s, 1, false), Some((0, 3))); // iw on "one"
        assert_eq!(vi_word_object(s, 1, true), Some((0, 5))); // aw takes trailing ws
        assert_eq!(vi_word_object(s, 3, false), Some((3, 5))); // iw on whitespace run
        assert_eq!(vi_word_object(s, 6, false), Some((5, 8))); // iw on "two"
        assert_eq!(vi_word_object(s, 8, false), Some((8, 9))); // the "." is its own object
        assert_eq!(vi_word_object(s, 8, true), Some((8, 9))); // aw: no ws on either side
        assert_eq!(vi_word_object(s, 20, false), Some((9, 14))); // clamps past-eol
        assert_eq!(vi_word_object("", 0, true), None);
    }

    #[test]
    fn vi_diw_and_caw_apply_operator_over_word_object() {
        let history: Vec<String> = Vec::new();
        // diw with the cursor inside "two" deletes just the word.
        let mut st = state("one two three", 5);
        let mut ring = VecDeque::new();
        for key in ['d', 'i', 'w'] {
            handle_vi_normal(&mut st, Key::Char(key), &history, &mut ring);
        }
        assert_eq!(st.buffer, "one  three");
        assert_eq!(ring.back().map(String::as_str), Some("two"));
        // daw takes the trailing space too.
        let mut st = state("one two three", 5);
        let mut ring = VecDeque::new();
        for key in ['d', 'a', 'w'] {
            handle_vi_normal(&mut st, Key::Char(key), &history, &mut ring);
        }
        assert_eq!(st.buffer, "one three");
        // caw drops to insert mode.
        let mut st = state("one two three", 5);
        st.vi_normal = true;
        let mut ring = VecDeque::new();
        for key in ['c', 'a', 'w'] {
            handle_vi_normal(&mut st, Key::Char(key), &history, &mut ring);
        }
        assert_eq!(st.buffer, "one three");
        assert!(!st.vi_normal);
    }

    #[test]
    fn vi_percent_jumps_and_deletes_inclusively() {
        let history: Vec<String> = Vec::new();
        let mut ring = VecDeque::new();
        // `%` on the open bracket jumps to its match.
        let mut st = state("f(a, b) g", 1);
        handle_vi_normal(&mut st, Key::Char('%'), &history, &mut ring);
        assert_eq!(st.cursor, 6);
        // `d%` backward from the close bracket deletes both brackets.
        let mut st = state("f(a, b) g", 6);
        handle_vi_normal(&mut st, Key::Char('d'), &history, &mut ring);
        handle_vi_normal(&mut st, Key::Char('%'), &history, &mut ring);
        assert_eq!(st.buffer, "f g");
    }

    #[test]
    fn vi_g_fetches_history_by_count() {
        let history = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        let mut ring = VecDeque::new();
        // No count: the most recent entry.
        let mut st = state("draft", 5);
        handle_vi_normal(&mut st, Key::Char('G'), &history, &mut ring);
        assert_eq!(st.buffer, "third");
        // 1G: the first entry; the draft is stashed for the return trip.
        let mut st = state("draft", 5);
        handle_vi_normal(&mut st, Key::Char('1'), &history, &mut ring);
        handle_vi_normal(&mut st, Key::Char('G'), &history, &mut ring);
        assert_eq!(st.buffer, "first");
        assert_eq!(st.draft, "draft");
    }

    #[test]
    fn delete_horizontal_space_removes_blanks_around_cursor() {
        let mut ring = VecDeque::new();
        let mut st = state("word   \t  next", 9);
        run_action(
            &mut st,
            EditorAction::DeleteHorizontalSpace,
            &Key::Alt('\\'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "wordnext");
        assert_eq!(st.cursor, 4);
        assert!(ring.is_empty(), "delete, not kill");
        // No blanks around the cursor: no-op.
        let mut st = state("word", 2);
        run_action(
            &mut st,
            EditorAction::DeleteHorizontalSpace,
            &Key::Alt('\\'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "word");
    }

    #[test]
    fn kill_whole_line_kills_regardless_of_cursor() {
        let mut ring = VecDeque::new();
        let mut st = state("entire line", 6);
        run_action(
            &mut st,
            EditorAction::KillWholeLine,
            &Key::Ctrl('x'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "");
        assert_eq!(ring.back().map(String::as_str), Some("entire line"));
    }

    #[test]
    fn operate_and_get_next_accepts_with_recall() {
        let mut ring = VecDeque::new();
        let mut st = state("echo two", 8);
        st.hist_index = Some(1); // recalled from history entry 1
        let after = run_action(
            &mut st,
            EditorAction::OperateAndGetNext,
            &Key::Ctrl('o'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert!(matches!(after, AfterKey::AcceptAndRecall));
    }

    #[test]
    fn history_ignore_space_skips_space_prefixed_lines() {
        let mut ed = Editor::new();
        ed.set_history_ignore_space(true);
        ed.add_history_entry(" secret");
        ed.add_history_entry("kept");
        assert_eq!(ed.history(), ["kept"]);
        // Off by default.
        let mut ed = Editor::new();
        ed.add_history_entry(" secret");
        assert_eq!(ed.history(), [" secret"]);
    }

    #[test]
    fn kill_ring_cap_rotates_oldest_out() {
        let mut ring = VecDeque::new();
        for i in 0..5 {
            push_ring(&mut ring, format!("k{i}"), 3);
        }
        assert_eq!(ring, ["k2", "k3", "k4"]);
        let mut ed = Editor::new();
        ed.set_max_kill_ring_len(0); // degenerate cap: nothing is kept
        push_ring(&mut ed.kill_ring, "x".to_string(), ed.cfg.max_kill_ring);
        assert!(ed.kill_ring.is_empty());
    }

    #[test]
    fn incremental_search_flags_a_failing_query() {
        let history = vec!["alpha".to_string(), "beta".to_string()];
        let mut st = state("", 0);
        st.cfg.bell = BellStyle::None;
        start_search(&mut st, &history, false);
        handle_search_key(&mut st, Key::Char('b'), &history).unwrap();
        let s = st.search.as_ref().unwrap();
        assert_eq!(s.hit, Some(1));
        assert!(!s.failed);
        handle_search_key(&mut st, Key::Char('z'), &history).unwrap();
        let s = st.search.as_ref().unwrap();
        assert!(s.failed, "query 'bz' matches nothing");
        assert_eq!(s.hit, Some(1), "last match stays shown, like readline");
        // Deleting back to a matching query clears the failure.
        handle_search_key(&mut st, Key::Backspace, &history).unwrap();
        assert!(!st.search.as_ref().unwrap().failed);
    }

    #[test]
    fn hint_is_computed_once_per_buffer() {
        use std::cell::Cell;
        struct Counting(Cell<usize>);
        impl Hooks for Counting {
            fn hint(&self, _line: &str, _history: &[String]) -> Option<String> {
                self.0.set(self.0.get() + 1);
                Some("int".to_string())
            }
        }
        let hooks = Counting(Cell::new(0));
        let mut st = state_hooked("pr", 2, &hooks);
        assert_eq!(cached_hint(&mut st, &[]), Some("int".to_string()));
        assert_eq!(cached_hint(&mut st, &[]), Some("int".to_string()));
        assert_eq!(hooks.0.get(), 1, "second lookup served from the cache");
        st.buffer.push('i'); // buffer changed: cache invalidates
        let _ = cached_hint(&mut st, &[]);
        assert_eq!(hooks.0.get(), 2);
    }

    #[test]
    fn paste_inserts_in_vi_normal_mode() {
        let history: Vec<String> = Vec::new();
        let mut ring = VecDeque::new();
        let mut st = state("ab", 1);
        st.vi = true;
        st.vi_normal = true;
        // The main loop routes Key::Paste around the normal-mode keymap;
        // insert_paste is the shared path both branches call.
        insert_paste(&mut st, "XY\r\nZ");
        assert_eq!(st.buffer, "aXY\nZb");
        // And the normal-mode keymap itself never sees pastes as keys.
        handle_vi_normal(&mut st, Key::Char('u'), &history, &mut ring);
    }

    #[test]
    fn back_tab_decodes_and_cycles_backward() {
        assert_eq!(csi_key("", b'Z'), Key::BackTab);
        assert_eq!(csi_key("1;2", b'Z'), Key::BackTab); // modifier params
        assert_eq!(parse_key_spec("\\e[Z"), Some(Key::BackTab));
        // A cold backward step starts on the *last* candidate and wraps.
        let cand = |s: &str| Candidate {
            display: s.to_string(),
            replacement: s.to_string(),
        };
        let mut st = state("say al", 6);
        st.menu = Some(MenuState {
            start: 4,
            inserted: 2,
            index: None,
            candidates: vec![cand("alpha"), cand("alphabet")],
        });
        menu_step(&mut st, true);
        assert_eq!(st.buffer, "say alphabet");
        menu_step(&mut st, true);
        assert_eq!(st.buffer, "say alpha");
        menu_step(&mut st, false); // and forward goes back again
        assert_eq!(st.buffer, "say alphabet");
    }

    #[test]
    fn display_width_skips_osc_sequences() {
        // OSC-8 hyperlink, BEL-terminated: only "link" is visible.
        assert_eq!(
            display_width("\x1b]8;;http://example.com\x07link\x1b]8;;\x07"),
            4
        );
        // OSC window title, ST-terminated.
        assert_eq!(display_width("\x1b]0;title\x1b\\ab"), 2);
        // CSI final bytes across the full 0x40–0x7E range still skip.
        assert_eq!(display_width("\x1b[38;5;196mred\x1b[0m"), 3);
    }

    #[test]
    fn visualize_keep_sgr_passes_color_and_transforms_the_rest() {
        // SGR passes through; tabs/newlines/control chars still render.
        // SGR is zero-width, so the tab after "red" pads from column 3.
        assert_eq!(
            visualize_keep_sgr("\x1b[31mred\x1b[0m\ta\n", 0),
            "\x1b[31mred\x1b[0m     a⏎"
        );
        // A non-SGR escape from a misbehaving hook is neutralized, not
        // sent to the terminal.
        assert_eq!(visualize_keep_sgr("\x1b[2J", 0), "^[[2J");
        assert_eq!(visualize_keep_sgr("plain", 0), "plain");
    }

    #[test]
    fn search_matchers_respect_ignore_case() {
        assert!(contains_match("Echo Foo", "foo", true));
        assert!(!contains_match("Echo Foo", "foo", false));
        assert!(starts_with_match("Echo Foo", "echo", true));
        assert!(!starts_with_match("Echo Foo", "echo", false));
        {
            let history = vec!["Git Status".to_string()];
            assert_eq!(find_match(&history, "git", history.len(), true), Some(0));
            assert_eq!(find_match(&history, "git", history.len(), false), None);
        }
    }

    #[test]
    fn vi_count_replace_and_paste() {
        let history: Vec<String> = Vec::new();
        let mut ring = VecDeque::new();
        // 3rx replaces three characters; the cursor lands on the last.
        let mut st = state("abcd", 0);
        st.cfg.bell = BellStyle::None;
        for key in ['3', 'r', 'x'] {
            handle_vi_normal(&mut st, Key::Char(key), &history, &mut ring);
        }
        assert_eq!(st.buffer, "xxxd");
        assert_eq!(st.cursor, 2);
        // Fewer than count characters left: fails outright, like vim.
        let mut st = state("ab", 0);
        st.cfg.bell = BellStyle::None;
        for key in ['3', 'r', 'x'] {
            handle_vi_normal(&mut st, Key::Char(key), &history, &mut ring);
        }
        assert_eq!(st.buffer, "ab");
        // 3p pastes three copies after the cursor character.
        let mut ring = VecDeque::from(vec!["xy".to_string()]);
        let mut st = state("ab", 0);
        for key in ['3', 'p'] {
            handle_vi_normal(&mut st, Key::Char(key), &history, &mut ring);
        }
        assert_eq!(st.buffer, "axyxyxyb");
    }

    #[cfg(unix)]
    #[test]
    fn history_file_is_private_and_replaced_atomically() {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("rusty_lines_perm_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut ed = Editor::new();
        ed.add_history_entry("secret");
        ed.save_history(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "new history file must be 0600");
        // No temp file left behind.
        let tmp = format!("{}.tmp.{}", path.display(), std::process::id());
        assert!(!std::path::Path::new(&tmp).exists());
        // An existing file keeps its (deliberately laxer) permissions.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        ed.add_history_entry("more");
        ed.save_history(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644, "existing permissions preserved");
        // append_history's create path is 0600 too.
        let _ = std::fs::remove_file(&path);
        ed.add_history_entry("tail");
        ed.append_history(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn search_exit_places_cursor_on_the_match() {
        assert_eq!(match_pos("echo --foo bar", "--foo", false), Some(5));
        assert_eq!(match_pos("Echo FOO", "foo", true), Some(5));
        assert_eq!(match_pos("abc", "zz", false), None);
        let history = vec!["echo --verbose x".to_string()];
        let mut st = state("", 0);
        st.cfg.bell = BellStyle::None;
        start_search(&mut st, &history, false);
        for c in "--v".chars() {
            handle_search_key(&mut st, Key::Char(c), &history).unwrap();
        }
        // Exiting with a non-search key keeps the match and puts the
        // cursor on it (readline's point), not at end-of-line.
        handle_search_key(&mut st, Key::Left, &history).unwrap();
        assert_eq!(st.buffer, "echo --verbose x");
        assert_eq!(st.cursor, 5);
    }

    #[test]
    fn undo_insert_groups_cap_at_20() {
        let mut st = state("", 0);
        // Simulate the main loop: type 45 characters one at a time.
        for i in 0..45 {
            let snapshot = (st.buffer.clone(), st.cursor);
            insert_char(&mut st, 'a');
            st.this_action = Action::Insert;
            record_undo(&mut st, snapshot);
            st.prev_action = st.this_action;
            let _ = i;
        }
        // 45 inserts → groups of 20/20/5 → 3 snapshots, not 1, not 45.
        assert_eq!(st.undo.len(), 3);
        undo_cmd(&mut st);
        assert_eq!(st.buffer.len(), 40, "one undo removes one 20-char group");
    }

    #[test]
    fn vi_delete_key_kills_into_the_ring() {
        let history: Vec<String> = Vec::new();
        let mut ring = VecDeque::new();
        let mut st = state("abc", 0);
        handle_vi_normal(&mut st, Key::Delete, &history, &mut ring);
        assert_eq!(st.buffer, "bc");
        assert_eq!(ring.back().map(String::as_str), Some("a"));
    }

    #[test]
    fn alt_multibyte_chords_decode() {
        assert_eq!(parse_key_spec("\\M-ö"), Some(Key::Alt('ö')));
        assert_eq!(parse_key_spec("\\eö"), Some(Key::Alt('ö')));
        // And the spec printer round-trips it.
        assert_eq!(
            parse_key_spec(&key_spec(&Key::Alt('ö'))),
            Some(Key::Alt('ö'))
        );
    }

    #[test]
    fn unique_completion_appends_the_append_character() {
        struct H;
        impl Hooks for H {
            fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
                (
                    0,
                    vec![Candidate {
                        display: "alpha".to_string(),
                        replacement: "alpha".to_string(),
                    }],
                )
            }
        }
        let mut ring = VecDeque::new();
        let mut st = state_hooked("al", 2, &H);
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "alpha ");
        assert_eq!(st.cursor, 6);
        // Disabled: the bare match is inserted.
        let mut st = state_hooked("al", 2, &H);
        st.cfg.completion_append = None;
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "alpha");
    }

    #[test]
    fn insert_completions_inserts_all_matches() {
        struct H;
        impl Hooks for H {
            fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
                let cand = |s: &str| Candidate {
                    display: s.to_string(),
                    replacement: s.to_string(),
                };
                (0, vec![cand("beta"), cand("alpha")])
            }
        }
        let mut ring = VecDeque::new();
        let mut st = state_hooked("a", 1, &H);
        run_action(
            &mut st,
            EditorAction::InsertCompletions,
            &Key::Alt('*'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "alpha beta"); // sorted, space-separated
        assert_eq!(st.cursor, 10);
    }

    #[test]
    fn resolve_recall_survives_index_shifts() {
        let hist = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Unshifted: the index still points at the expected entry.
        let h = hist(&["a", "b", "c"]);
        assert_eq!(resolve_recall(&h, 1, "b"), Some(1));
        // erasedups removed an earlier duplicate: re-locate by content.
        let h = hist(&["b", "c", "a"]);
        assert_eq!(resolve_recall(&h, 1, "b"), Some(0));
        // The entry vanished entirely: no recall.
        let h = hist(&["c"]);
        assert_eq!(resolve_recall(&h, 1, "b"), None);
    }

    #[test]
    fn paste_during_search_types_into_the_query() {
        let history = vec!["echo --verbose".to_string()];
        let mut st = state("", 0);
        st.cfg.bell = BellStyle::None;
        start_search(&mut st, &history, false);
        let outcome =
            handle_search_key(&mut st, Key::Paste("--verb\n".to_string()), &history).unwrap();
        assert!(matches!(outcome, SearchOutcome::Continue));
        let s = st.search.as_ref().unwrap();
        assert_eq!(s.query, "--verb");
        assert_eq!(s.hit, Some(0));
    }

    #[test]
    fn search_starts_from_the_current_history_position() {
        let history = vec!["git alpha".to_string(), "git beta".to_string()];
        // Recalled (unedited) entry 0: the search is seeded there, so a
        // query matching both entries stays on 0 instead of jumping to
        // the newest match.
        let mut st = state("git alpha", 9);
        st.hist_index = Some(0);
        start_search(&mut st, &history, false);
        assert_eq!(st.search.as_ref().unwrap().hit, Some(0));
        handle_search_key(&mut st, Key::Char('g'), &history).unwrap();
        assert_eq!(st.search.as_ref().unwrap().hit, Some(0));
        // An *edited* recall must not seed (exiting a seeded search
        // rewrites the buffer from history, clobbering the edits).
        let mut st = state("git alpha EDITED", 16);
        st.hist_index = Some(0);
        start_search(&mut st, &history, false);
        assert_eq!(st.search.as_ref().unwrap().hit, None);
    }

    #[test]
    fn malformed_hook_offsets_are_clamped_not_panics() {
        struct Evil;
        impl Hooks for Evil {
            fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
                (
                    usize::MAX, // way past the cursor
                    vec![Candidate {
                        display: "x".to_string(),
                        replacement: "x".to_string(),
                    }],
                )
            }
            fn expand_abbreviation(&self, _line: &str, _cursor: usize) -> Option<(usize, String)> {
                Some((usize::MAX, "gone".to_string()))
            }
        }
        let mut ring = VecDeque::new();
        // Completion with an out-of-range start: clamped to the cursor.
        let mut st = state_hooked("ab", 2, &Evil);
        st.cfg.completion_append = None;
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "abx");
        // Abbreviation expansion likewise.
        let mut st = state_hooked("ab", 2, &Evil);
        run_action(
            &mut st,
            EditorAction::SelfInsert,
            &Key::Char(' '),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "abgone ");
        // And a mid-character offset lands on the boundary below it.
        assert_eq!(clamp_start("aü", 2, 3), 1);
    }

    #[test]
    fn action_names_round_trip() {
        for (_, action) in DEFAULT_BINDINGS {
            assert_eq!(
                EditorAction::from_name(action.name()),
                Some(*action),
                "{} does not round-trip",
                action.name()
            );
        }
        // Actions with no default key still have names.
        for action in [
            EditorAction::SelfInsert,
            EditorAction::KillWholeLine,
            EditorAction::MenuComplete,
            EditorAction::CharacterSearchBackward,
            EditorAction::EditAndExecuteCommand,
        ] {
            assert_eq!(EditorAction::from_name(action.name()), Some(action));
        }
        assert_eq!(EditorAction::from_name("no-such-command"), None);
    }

    #[test]
    fn host_bindings_are_listable() {
        let mut ed = Editor::new();
        ed.bind_host("\\C-g", "fzf".to_string()).unwrap();
        ed.bind("\\C-f", EditorAction::KillLine).unwrap();
        let hosts: Vec<(String, &str)> = ed.host_bindings().collect();
        assert_eq!(hosts, [("\\C-g".to_string(), "fzf")]);
        // Action bindings stay out of the host listing and vice versa.
        assert!(ed.bindings().any(|(k, _)| k == "\\C-f"));
        assert!(ed.bindings().all(|(k, _)| k != "\\C-g"));
    }

    #[test]
    fn back_tab_completes_in_vi_normal_mode_too() {
        assert!(key_completes(&Key::BackTab, &[], true));
        assert!(key_completes(&Key::Tab, &[], true));
        assert!(!key_completes(&Key::Char('x'), &[], true));
    }

    /// A tiny deterministic xorshift — the chaos tests below need
    /// repeatable garbage, not real randomness.
    fn xorshift(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    #[test]
    fn decoders_survive_byte_soup() {
        // The pure decoders and text helpers slice and do arithmetic on
        // arbitrary input; hammer them with seeded garbage and require
        // only that nothing panics. (Cheap always-on fuzzing — no
        // cargo-fuzz dependency, per the crate's zero-dep ethos.)
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for round in 0..2000u32 {
            let len = (xorshift(&mut seed) % 12) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| (xorshift(&mut seed) & 0xff) as u8)
                .collect();
            let _ = decode_key_bytes(&bytes);
            let _ = csi_key(
                std::str::from_utf8(&bytes).unwrap_or(""),
                (xorshift(&mut seed) & 0xff) as u8,
            );
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let _ = parse_key_spec(&s);
            let _ = display_width(&s);
            let _ = longest_common_prefix(&[&s, "probe"]);
            let _ = common_prefix(&[&s, "aB"], true);
            let _ = contains_match(&s, "aB", true);
            let _ = starts_with_match(&s, &s, true);
            let _ = match_pos(&s, "aB", round % 2 == 0);
            {
                let col = (xorshift(&mut seed) % 40) as usize;
                let _ = visualize(&s, col);
                let _ = visualize_marked(&s, col, (xorshift(&mut seed) % 16) as usize);
                let _ = visualize_keep_sgr(&s, col);
            }
            for i in (0..=s.len()).filter(|&i| s.is_char_boundary(i)) {
                let _ = word_back(&s, i);
                let _ = word_back_alnum(&s, i);
                let _ = word_forward_alnum(&s, i);
                let _ = vi_word_fwd(&s, i);
                let _ = vi_word_back(&s, i);
                let _ = vi_word_end(&s, i);
                let _ = vi_match_bracket(&s, i);
                let _ = vi_word_object(&s, i, true);
                let _ = vi_word_object(&s, i, false);
                {
                    let _ = vi_find_target(&s, i, 'f', 'x');
                    let _ = vi_find_target(&s, i, 'T', 'x');
                    let _ = clamp_start(&s, xorshift(&mut seed) as usize, i);
                }
            }
        }
    }

    #[test]
    fn history_entries_that_look_like_comments_round_trip() {
        let path =
            std::env::temp_dir().join(format!("rusty_lines_hash_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // Short `#digits` comment entries survive a plain round-trip
        // (interactive shells store comment lines).
        let mut ed = Editor::new();
        ed.add_history_entry("#42");
        ed.add_history_entry("echo hi");
        ed.save_history(&path).unwrap();
        let mut ed2 = Editor::new();
        ed2.load_history(&path).unwrap();
        assert_eq!(ed2.history(), ["#42", "echo hi"]);
        // And through the timestamped format: the stamp pairs with the
        // stamp-shaped entry instead of eating it.
        let mut ed = Editor::new();
        ed.set_history_timestamps(true);
        ed.add_history_entry("#1234567890");
        ed.add_history_entry("echo hi");
        ed.save_history(&path).unwrap();
        let mut ed3 = Editor::new();
        ed3.load_history(&path).unwrap();
        assert_eq!(ed3.history(), ["#1234567890", "echo hi"]);
        // A dangling epoch-scale stamp at EOF is an entry, not a stamp.
        std::fs::write(&path, "echo a\n#987654321\n").unwrap();
        let mut ed4 = Editor::new();
        ed4.load_history(&path).unwrap();
        assert_eq!(ed4.history(), ["echo a", "#987654321"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn public_types_have_useful_derives() {
        // ReadResult: comparable and printable, so hosts can assert on it.
        assert_eq!(
            ReadResult::Line("x".to_string()),
            ReadResult::Line("x".to_string())
        );
        assert_ne!(ReadResult::Eof, ReadResult::Interrupted);
        let cloned = ReadResult::TimedOut.clone();
        assert!(format!("{cloned:?}").contains("TimedOut"));
        // Candidate: clonable and printable.
        let c = Candidate {
            display: "d".to_string(),
            replacement: "r".to_string(),
        };
        assert!(format!("{:?}", c.clone()).contains("replacement"));
        // Editor: printable (host debug logs).
        assert!(format!("{:?}", Editor::new()).contains("history"));
    }

    #[test]
    fn double_esc_decodes_as_escape() {
        // The streaming decoder's pure twin can't express the timing
        // path, but the spec parser shares the table for single ESC…
        assert_eq!(parse_key_spec("\\e"), Some(Key::Esc));
        // …and the raw-mode recipe checker matches what apply_raw_flags
        // produces (the self-healing tick depends on this agreement).
        let mut t = term_sys::tcgetattr_stdin().unwrap_or_else(|_| unsafe { std::mem::zeroed() });
        term_sys::apply_raw_flags(&mut t);
        assert!(term_sys::is_raw(&t));
    }

    #[test]
    fn undo_coalesces_delete_runs_like_insert_runs() {
        // Type 5 chars, then rub them all out one Backspace at a time:
        // the deletions must undo as one group, not five.
        let mut st = state("", 0);
        let mut ring = VecDeque::new();
        for _ in 0..5 {
            let snapshot = (st.buffer.clone(), st.cursor);
            insert_char(&mut st, 'a');
            st.this_action = Action::Insert;
            record_undo(&mut st, snapshot);
            st.prev_action = st.this_action;
        }
        for _ in 0..5 {
            let snapshot = (st.buffer.clone(), st.cursor);
            st.this_action = Action::Other;
            run_action(
                &mut st,
                EditorAction::BackwardDeleteChar,
                &Key::Backspace,
                &[],
                &mut ring,
            )
            .unwrap();
            record_undo(&mut st, snapshot);
            st.prev_action = st.this_action;
        }
        assert_eq!(st.buffer, "");
        // One insert group + one delete group.
        assert_eq!(st.undo.len(), 2);
        undo_cmd(&mut st);
        assert_eq!(st.buffer, "aaaaa", "one undo restores the whole rubout");
    }

    #[test]
    fn search_backspace_stays_position_disciplined() {
        let history = vec!["git alpha".to_string(), "git beta".to_string()];
        let mut st = state("git alpha", 9);
        st.cfg.bell = BellStyle::None;
        st.hist_index = Some(0);
        start_search(&mut st, &history, false);
        handle_search_key(&mut st, Key::Char('g'), &history).unwrap();
        handle_search_key(&mut st, Key::Char('i'), &history).unwrap();
        assert_eq!(st.search.as_ref().unwrap().hit, Some(0));
        // Shrinking the query must not teleport the hit to the newest
        // match; it re-searches at-or-before the current position.
        handle_search_key(&mut st, Key::Backspace, &history).unwrap();
        assert_eq!(st.search.as_ref().unwrap().hit, Some(0));
    }

    #[test]
    fn dedup_load_matches_sequential_semantics_and_scales() {
        let path =
            std::env::temp_dir().join(format!("rusty_lines_dedup_load_{}", std::process::id()));
        std::fs::write(&path, "a\nb\na\nc\nb\na\n").unwrap();
        // Batch dedup load == what sequential erasedups adds produce.
        let mut ed = Editor::new();
        ed.set_history_dedup(true);
        ed.add_history_entry("c");
        ed.add_history_entry("x");
        ed.load_history(&path).unwrap();
        assert_eq!(ed.history(), ["x", "c", "b", "a"]);
        // And a large file loads without the quadratic scan (smoke: this
        // finishes instantly at O(n); the old path did ~10^8 compares).
        let big: String = (0..20_000).map(|i| format!("cmd {}\n", i % 500)).collect();
        std::fs::write(&path, big).unwrap();
        let mut ed = Editor::new();
        ed.set_history_dedup(true);
        ed.load_history(&path).unwrap();
        assert_eq!(ed.history().len(), 500);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn every_decodable_key_round_trips_through_its_spec() {
        // The property behind the hand-picked spec tests: any key the
        // decoder can produce has a spec that parses back to itself.
        let mut keys = Vec::new();
        for b in 0..=255u8 {
            keys.extend(decode_key_bytes(&[b]));
            keys.extend(decode_key_bytes(&[0x1b, b]));
            keys.extend(decode_key_bytes(&[0x1b, b'[', b]));
            keys.extend(decode_key_bytes(&[0x1b, b'O', b]));
        }
        assert!(keys.len() > 200, "decoder sweep looks broken");
        for key in keys {
            assert_eq!(
                parse_key_spec(&key_spec(&key)).as_ref(),
                Some(&key),
                "spec {:?} for {key:?} does not round-trip",
                key_spec(&key)
            );
        }
    }

    #[test]
    fn action_display_is_the_readline_name() {
        assert_eq!(format!("{}", EditorAction::KillLine), "kill-line");
        assert_eq!(
            EditorAction::MenuCompleteBackward.to_string(),
            "menu-complete-backward"
        );
        // NoHooks is embeddable: Debug + Clone + Copy + Default.
        let h: NoHooks = Default::default();
        let _copy = h;
        assert_eq!(format!("{h:?}"), "NoHooks");
    }

    #[test]
    fn public_types_are_send_and_sync() {
        // Hosts move the editor across threads (job control, async
        // wrappers); a refactor must not silently break this.
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Editor>();
        assert_sync::<Editor>();
        assert_send::<ReadResult>();
        assert_send::<Candidate>();
        assert_send::<EditorAction>();
        assert_send::<NoHooks>();
    }

    #[test]
    fn history_edits_stash_and_restore() {
        let history = vec!["one".to_string(), "two".to_string()];
        let mut st = state("draft", 5);
        st.cfg.bell = BellStyle::None;
        // Up to "two", edit it, Up to "one", back Down: edit preserved.
        history_prev(&mut st, &history);
        assert_eq!(st.buffer, "two");
        st.buffer.push('X');
        st.cursor = st.buffer.len();
        history_prev(&mut st, &history);
        assert_eq!(st.buffer, "one");
        history_next(&mut st, &history);
        assert_eq!(st.buffer, "twoX", "edit lost on navigation");
        // Reverting the edit clears the stash.
        st.buffer = "two".to_string();
        history_prev(&mut st, &history);
        history_next(&mut st, &history);
        assert_eq!(st.buffer, "two");
        // And the draft still comes back at the end.
        history_next(&mut st, &history);
        assert_eq!(st.buffer, "draft");
    }

    #[test]
    fn search_moves_the_history_position_and_interrupts() {
        let history = vec!["alpha".to_string(), "beta".to_string()];
        // Exiting a search on a hit sets the history position there, so
        // a following Up walks backward from the found entry.
        let mut st = state("", 0);
        st.cfg.bell = BellStyle::None;
        start_search(&mut st, &history, false);
        handle_search_key(&mut st, Key::Char('b'), &history).unwrap();
        handle_search_key(&mut st, Key::Left, &history).unwrap();
        assert_eq!(st.buffer, "beta");
        assert_eq!(st.hist_index, Some(1));
        history_prev(&mut st, &history);
        assert_eq!(st.buffer, "alpha", "Up should continue from the hit");
        // C-c during search aborts the whole read, like bash.
        let mut st = state("", 0);
        st.cfg.bell = BellStyle::None;
        start_search(&mut st, &history, false);
        let outcome = handle_search_key(&mut st, Key::Ctrl('c'), &history).unwrap();
        assert!(matches!(outcome, SearchOutcome::Interrupt));
        // C-g aborts just the search, restoring the pre-search line.
        let mut st = state("mine", 4);
        st.cfg.bell = BellStyle::None;
        start_search(&mut st, &history, false);
        handle_search_key(&mut st, Key::Char('b'), &history).unwrap();
        let outcome = handle_search_key(&mut st, Key::Ctrl('g'), &history).unwrap();
        assert!(matches!(outcome, SearchOutcome::Exit));
        assert_eq!(st.buffer, "mine");
        // And C-g in normal editing is readline's abort (named, bound).
        assert_eq!(EditorAction::from_name("abort"), Some(EditorAction::Abort));
        assert_eq!(default_action(&Key::Ctrl('g')), Some(EditorAction::Abort));
    }

    #[test]
    fn alt_f_accepts_one_hint_word() {
        struct H;
        impl Hooks for H {
            fn hint(&self, line: &str, _history: &[String]) -> Option<String> {
                (line == "he").then(|| "llo world".to_string())
            }
        }
        let mut ring = VecDeque::new();

        // At end of line: accept exactly one word of the hint.
        let mut st = state_hooked("he", 2, &H);
        run_action(
            &mut st,
            EditorAction::ForwardWord,
            &Key::Alt('f'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "hello");
        assert_eq!(st.cursor, 5);

        // Mid-line: plain word motion, no hint involvement.
        let mut st = state_hooked("he", 0, &H);
        run_action(
            &mut st,
            EditorAction::ForwardWord,
            &Key::Alt('f'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "he");
        assert_eq!(st.cursor, 2);
    }

    /// Fill [`STDIN_BUF`] as if these bytes had just arrived off fd 0 —
    /// lets a test drive `read_byte`/`read_key` (and anything built on
    /// them, like `quoted_insert`) without a real terminal.
    fn feed_stdin(bytes: &[u8]) {
        STDIN_BUF.with(|b| {
            let (data, pos) = &mut *b.borrow_mut();
            data.clear();
            data.extend_from_slice(bytes);
            *pos = 0;
        });
    }

    #[test]
    fn quoted_insert_inserts_the_raw_control_byte() {
        // C-v C-a: the ^A byte lands in the buffer literally instead of
        // running beginning-of-line, and renders ^X-style.
        let mut st = state("", 0);
        let mut ring = VecDeque::new();
        feed_stdin(&[0x01]);
        run_action(
            &mut st,
            EditorAction::QuotedInsert,
            &Key::Ctrl('v'),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "\u{1}");
        assert_eq!(st.cursor, 1);
        assert_eq!(visualize(&st.buffer, 0), "^A");
    }

    #[test]
    fn unrecognized_csi_sequence_is_consumed_whole() {
        // An SGR mouse report (ESC[<65;5;10M): the `<` intermediate takes
        // the private-mode/intermediate path, so the whole sequence must
        // be swallowed as one Key::Other instead of leaking its tail
        // ("65;5;10M") into the buffer as typed text. The plain 'x' right
        // after proves nothing leaked.
        feed_stdin(b"\x1b[<65;5;10Mx");
        assert_eq!(read_key(&NoHooks).unwrap(), Some(Key::Other));
        assert_eq!(read_key(&NoHooks).unwrap(), Some(Key::Char('x')));
    }

    #[test]
    fn load_history_skips_the_v2_header() {
        let path =
            std::env::temp_dir().join(format!("rusty_lines_v2_header_test_{}", std::process::id()));
        std::fs::write(&path, "#V2\nfoo\nbar\n").unwrap();
        let mut ed = Editor::new();
        ed.load_history(&path).unwrap();
        assert_eq!(ed.history(), &["foo", "bar"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn abbreviation_expands_on_space() {
        struct H;
        impl Hooks for H {
            fn expand_abbreviation(&self, line: &str, cursor: usize) -> Option<(usize, String)> {
                (&line[..cursor] == "gco").then(|| (0, "git checkout".to_string()))
            }
        }
        let mut st = state_hooked("gco", 3, &H);
        let mut ring = VecDeque::new();
        run_action(
            &mut st,
            EditorAction::SelfInsert,
            &Key::Char(' '),
            &[],
            &mut ring,
        )
        .unwrap();
        assert_eq!(st.buffer, "git checkout ");
        assert_eq!(st.cursor, 13);
    }

    #[test]
    fn completion_ignore_case_matches_regardless_of_case() {
        struct H;
        impl Hooks for H {
            fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
                let cand = |s: &str| Candidate {
                    display: s.to_string(),
                    replacement: s.to_string(),
                };
                (0, vec![cand("Echo"), cand("echelon")])
            }
        }
        let mut ring = VecDeque::new();
        // Off (default): 'E' vs 'e' differ, so there is no common prefix
        // to insert.
        let mut st = state_hooked("", 0, &H);
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "");
        // On: the case-insensitive common prefix "Ech" (first candidate's
        // spelling) is inserted.
        let mut st = state_hooked("", 0, &H);
        st.cfg.completion_ignore_case = true;
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "Ech");
    }

    #[test]
    fn show_all_if_ambiguous_lists_immediately_on_first_tab() {
        struct H;
        impl Hooks for H {
            fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
                let cand = |s: &str| Candidate {
                    display: s.to_string(),
                    replacement: s.to_string(),
                };
                (0, vec![cand("alpha"), cand("alphabet")])
            }
        }
        let mut ring = VecDeque::new();
        // Off (default): the first Tab only inserts the common prefix;
        // the list waits for a second Tab (no progress next time).
        let mut st = state_hooked("", 0, &H);
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "alpha");
        assert!(st.menu.is_none());
        // On: the same first Tab both inserts the prefix and lists (arms
        // the menu) right away.
        let mut st = state_hooked("", 0, &H);
        st.cfg.show_all_if_ambiguous = true;
        run_action(&mut st, EditorAction::Complete, &Key::Tab, &[], &mut ring).unwrap();
        assert_eq!(st.buffer, "alpha");
        assert!(st.menu.is_some());
    }
}
