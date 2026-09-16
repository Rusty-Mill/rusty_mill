# rusty_lines

[![CI](https://github.com/baileyrd/rusty_lines/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_lines/actions/workflows/ci.yml)

A hand-rolled line editor for Rust — a readline alternative with no
dependency on readline, ncurses, or any editing crate. The only library
dependency is `unicode-width` (display columns); the terminal syscalls
come from the hand-rolled [`rusty_libc`] backend on Linux (no
third-party code at all) or the `libc` crate on other Unix. Grown
inside the [rush shell](https://github.com/baileyrd/rush) as its
`rustyline` replacement, then extracted.

[`rusty_libc`]: https://github.com/baileyrd/rusty_libc

```rust
use rusty_lines::{Editor, NoHooks, ReadResult};

let mut ed = Editor::new();
match ed.read_line("prompt> ", "right-prompt", &NoHooks)? {
    ReadResult::Line(line) => { /* … */ }
    ReadResult::Interrupted => { /* Ctrl-C */ }
    ReadResult::Eof => { /* Ctrl-D on an empty line */ }
}
```

The host integrates through the `Hooks` trait — every method has a no-op
default, so `&NoHooks` gives plain editing:

```rust
pub trait Hooks {
    fn complete(&self, line: &str, pos: usize) -> (usize, Vec<Candidate>);
    fn hint(&self, line: &str, history: &[String]) -> Option<String>;
    fn highlight(&self, line: &str) -> String;
    fn expand_abbreviation(&self, line: &str, cursor: usize) -> Option<(usize, String)>;
    fn vi_mode(&self) -> bool;              // checked live, per read_line
    fn external_editor(&self) -> Option<String>; // C-x C-e; falls back $VISUAL/$EDITOR/vi
    fn on_interrupted_read(&self);          // EINTR, and every idle tick: fire pending signal traps
    fn host_binding(&self, tag: &str, line: &mut String, cursor: &mut usize); // bind -x
}
```

A hook that wants to print its own output from `on_interrupted_read`
(a shell reporting a background job's completion while idle at the
prompt, say) calls `prepare_external_output()` first — see its own doc
comment for why the ordering matters.

Try it: `cargo run --example demo` — or `hooked` (completion, hints,
host bindings), `vi` (vi mode + mode indicator), `rprompt`,
`initial`, `timeout`, and `notify` (`prepare_external_output`).

## Benchmarks

`bench/` is a standalone (non-workspace, non-published) harness that
drives rusty_lines, rustyline, and reedline side by side under a real
pseudo-terminal with identical byte streams — per-keystroke latency,
burst throughput, paste ingestion, history navigation, and bytes
painted per keystroke (what a slow ssh link pays for):

```sh
cd bench && cargo run --release
```

It is deliberately not a workspace member so the editor itself stays
dependency-free; the harness pulls in the competitors on demand.

## Feature matrix

The feature set was audited against the line editors in wide use — GNU
readline, libedit, zsh ZLE, fish, ksh93's emacs/vi modes, linenoise,
replxx, rustyline, reedline, prompt_toolkit/PSReadLine — and the gaps
closed. References are to the editor whose behavior is matched. The
full per-editor survey behind this table is in
[docs/survey.md](docs/survey.md).

| Capability | Reference behavior |
|---|---|
| Emacs basics: C-a/C-e/C-b/C-f, C-d, C-h, C-t, arrows, Home/End/Delete | readline, everywhere |
| Kill ring: C-k, C-u, C-w, M-d, M-Backspace kill *into* a ring; C-y yanks; M-y rotates; consecutive kills grow one entry (append forward / prepend backward); ring survives across lines; depth configurable (`set_max_kill_ring_len`, and `set_max_undo_len` for undo) | readline, ZLE, fish |
| More kill/delete commands: `kill-whole-line` (unbound, like readline) and `delete-horizontal-space` (M-\, a delete — nothing enters the ring) | readline |
| Operate-and-get-next: C-o accepts the line and pre-loads the next `read_line` with the history entry after it, for replaying command sequences; the recalled entry is re-located by content if `erasedups` or a host history edit shifted indices in between | readline `operate-and-get-next`, bash |
| Pre-seeded lines: `read_line_with_initial((left, right))` starts the edit with text in the buffer and the cursor between the halves — `fc`-style edit-and-rerun, offered corrections; combinable with the deadline via `read_line_with_initial_timeout` | rustyline `readline_with_initial`, zsh `print -z` |
| Word flavors: M-b/M-f/M-d/M-Backspace use alphanumeric words, C-w whitespace words (unix-word-rubout) | readline's two word classes |
| Ctrl-arrow / Alt-arrow word motion (`CSI 1;5C` etc.) | every modern terminal editor |
| Undo: C-_ , C-x C-u (and C-z, fish-style); runs of self-insert *and of single-character deletes* undo in groups of at most 20 characters (readline's chunking); M-r reverts the whole line | readline (incl. `revert-line`), ZLE, fish |
| Transpose: C-t chars (two-before at EOL), M-t words | readline |
| Case ops: M-u / M-l / M-c | readline, ZLE |
| Insert last argument: M-. / M-_ , repeat cycles older entries | readline, ZLE |
| Quoted insert: C-v / C-q; control chars render `^X`-style | readline |
| Edit line in `$VISUAL`/`$EDITOR`: C-x C-e (emacs), `v` (vi normal); result returned as the line | readline, ZLE, fish (Alt-e) |
| History: Up/Down with draft preservation, C-p/C-n, M-< / M->; in-session edits to recalled entries survive navigating away and back, reverting once the line is accepted | readline; zsh (edit persistence scope) |
| History cap: `set_max_history_len` drops oldest past the limit | readline `stifle_history`, bash `HISTSIZE` |
| History dedup option: `set_history_dedup` erases earlier duplicates | bash `HISTCONTROL=erasedups`, fish |
| History ignore-space option: `set_history_ignore_space` skips lines starting with a space | bash `HISTCONTROL=ignorespace`, zsh `HIST_IGNORE_SPACE` |
| History persistence: `save_history` rewrites atomically (temp file + rename) and creates the file mode 0600, `append_history` appends only new entries; `load_history` tolerates a rustyline `#V2` header, and `#<digits>` *comment entries* round-trip (only epoch-scale, paired stamps parse as timestamps) | bash `histappend` and its 0600 history file; rustyline migration |
| Clear screen: C-l clears and repaints the edit region at the top | readline `clear-screen` |
| Incremental search: C-r backward *and* C-s forward (IXON is off), direction switching mid-search; starts from the current history position after an unedited recall and the found entry *becomes* the position on exit; a paste types into the query; a miss shows `(failed reverse-i-search)` and rings the bell, keeping the last match visible; leaving the search puts the cursor *on* the match (readline's point); C-g aborts with a bell, C-c aborts the whole read | readline, ZLE (bash paste-into-query, `^C` abort) |
| Case-insensitive search option: `set_search_ignore_case` covers incremental *and* prefix search | readline 8.1 `search-ignore-case` |
| Bell on failed operations: no-match completion, history past either end, failed prefix search, failed vi find — all per `set_bell_style` | readline `bell-style` |
| Prefix history search: PageUp/PageDown, M-p/M-n; the prefix re-anchors on the buffer up to the cursor whenever the previous key wasn't a prefix search | ZLE `history-beginning-search`, fish Up, PSReadLine |
| History hints (autosuggestions) via `Hooks::hint`, Right/End accepts; M-f / Ctrl-Right at end of line accepts one word | fish, PSReadLine, linenoise hints |
| Syntax highlighting while typing via `Hooks::highlight`: the hook paints the *raw* buffer (true text, true byte offsets); its SGR markup passes through and the editor re-applies control-char visualization around it | fish, ZLE plugins, replxx |
| Tab completion via `Hooks::complete`: LCP insertion + sorted, column-major candidate list; big lists ask `Display all N possibilities? (y or n)` first (`set_completion_query_items`, default 100); a unique match gets the append character — a space by default (`set_completion_append_character`) | readline `CompletionType::List`, `completion-query-items`, `rl_completion_append_character` |
| Menu cycling: Tab after the candidate list walks the candidates in-line, wrapping around; Shift-Tab (`CSI Z`) cycles backward, starting from the last candidate | zsh `AUTO_MENU` / `reverse-menu-complete`, readline `menu-complete(-backward)` |
| `possible-completions` (M-?) lists the candidates without editing the buffer; `insert-completions` (M-*) inserts every match, space-separated | readline |
| Character search: C-] reads a character and moves the cursor to its next occurrence (`character-search-backward` available unbound) | readline |
| Abbreviation expansion on space via `Hooks::expand_abbreviation` | fish `abbr` |
| Right-side prompt (second `read_line` argument), hidden when the line grows into it | zsh `$RPS1`, fish, reedline |
| Bracketed paste: paste arrives as one event — tabs/ESC insert literally, nothing executes until Enter; inserts literally in vi normal mode too; multi-line pastes keep their newlines (shown `⏎`) and return as a unit; multi-line history entries stored joined with `; ` (bash `cmdhist`) | readline 8.1+, ZLE, fish, reedline |
| vi mode (`Hooks::vi_mode`): counts (motions and `x ~ r p P` — `3rx`, `3p`); `d`/`c`/`y` operators over motions; `h l 0 ^ $ w W b B e E f F t T ; , %`; the `iw`/`aw` text objects; `x X D C s S Y r ~ p P u` (Delete ≡ `x`); `i I a A`; `k`/`j` history, `G` fetches by count; `cw`≡`ce` quirk; Esc backs the cursor up one | readline vi mode, ksh, ZLE; vim (`%`, `G`, `iw`/`aw`, counts) |
| Mode indicator: `set_show_mode_in_prompt` prefixes `(ins)`/`(cmd)` (vi) or `@` (emacs) to the prompt | readline `show-mode-in-prompt` and its default mode strings |
| Wide chars + UTF-8 input assembly (including Alt + non-ASCII chords, `\M-ö`); ANSI-aware width math (CSI *and* OSC — a hyperlinked or titled prompt measures correctly); soft-wrap repaint without flicker (paint-then-clear); `^X` control-char visualization keeps cursor math exact; tabs render at 8-column stops of the true display offset | all modern |
| Multi-line prompts (`PS1` with newlines): the lines before the last paint once per region; only the final line is the repainted edit row | readline, zsh |
| Robust escape decoding: unrecognized CSI sequences (SGR mouse reports, private modes) are consumed whole per ECMA-48 instead of leaking their tail into the buffer as typed text | readline, all modern |
| Resize: width re-read on every repaint; a resize while idle at the prompt repaints within a poll tick (~200ms); the same tick re-asserts raw mode if an external SIGTSTP/`fg` or a host command's `stty` left the terminal cooked | readline SIGWINCH + SIGCONT re-preparation, approximated without signals |
| Key rebinding: `bind`/`unbind` accept readline key-spec spellings (`\C-x`, `\M-f`, `\e[1;5C`), remapping single keys to named `EditorAction`s; `bindings()` lists the effective keymap; `EditorAction::name`/`from_name` map actions to readline's command names so hosts don't keep their own table | readline `bind '"\C-x": kill-line'`, `bind -P`, `bind -r`, `rl_named_function` |
| Host-command bindings: `bind_host` + `Hooks::host_binding` — raw mode suspends, the host runs its command against the line/cursor, the edit resumes; `host_bindings()` lists them | bash `bind -x`, `bind -X` (`READLINE_LINE`/`READLINE_POINT`) |
| Readline variables: `set_completion_ignore_case`, `set_show_all_if_ambiguous`, `set_menu_complete`, `set_bell_style` | readline `set completion-ignore-case on` … |
| Read deadline: `read_line_timeout` returns `ReadResult::TimedOut` when no complete line arrives in time | bash `$TMOUT`, readline `rl_set_timeout` |
| History timestamps: `#<epoch>` comment lines, written only under `set_history_timestamps`, always parsed on load (both formats round-trip); `history_timestamps()` exposes them | bash `HISTTIMEFORMAT` file format |
| In-place history replacement: `replace_history` resyncs the list after a host's history edits without rebuilding the editor (kill ring and session state survive) | bash `history -c` / `history -d` support |
| Terminal facilities: `terminal_size()` (cols, rows) and `with_echo_disabled` (panic-safe echo-off around a closure) | bash `checkwinsize` `$COLUMNS`/`$LINES`; `read -s` |

## Deliberate narrowings

Checked against the same field and consciously not modeled — each is
either niche, terminal-hostile, or a different program's job:

- **Multi-line *buffer editing*** (zsh/fish/reedline edit a `\n`-separated
  buffer with per-line cursor movement). The buffer is one logical line;
  embedded newlines (from a paste or C-v C-j) render as `⏎` and return
  correctly, but Up/Down navigate history, not buffer rows. C-x C-e hands
  real multi-line editing to `$EDITOR`.
- **Full keymap programmability** (readline's `.inputrc`, ZLE widgets,
  fish's `bind` functions). Single-key rebinding of the named actions and
  host-command bindings *are* supported (see the matrix — revisiting a
  narrowing); what stays declined: user-defined widgets, multi-key chord
  bindings beyond the built-in C-x set, rebinding vi normal mode, and
  `.inputrc` file parsing (the host's `bind` builtin passes specs through).
- **Keyboard macros** (readline C-x `(` … `)`), **numeric arguments in
  emacs mode** (M-digit; vi counts are supported), **mark/region**
  (C-@, C-x C-x), **redo** (readline has none either).
- **vi registers, `.` repeat, `/` history search** (C-r covers search from
  insert mode; the unnamed register is the kill ring).
- **Completion paging and menu-select UI** (fish's pager, ZLE's
  interactive menu with a highlighted selection): long candidate lists
  print unpaged — though readline's `completion-query-items` y/n guard
  (see the matrix) asks before dumping a big one; repeated-Tab cycling
  stands in for menu-select.
- **Signal-driven resize repaint** (readline installs a SIGWINCH
  handler). The width is re-read from the tty on every repaint, and a
  resize while idle at the prompt is noticed by the input poll tick and
  repainted; only the signal handler itself is declined — installing
  one from a library is the host's business, not the editor's.
- **Grapheme-cluster cursor math** (combining marks, emoji ZWJ
  sequences). Width is per-`char` via `unicode-width`; getting clusters
  right would add a segmentation dependency while terminals themselves
  disagree on cluster widths, so the common-case behavior is kept.
- **Non-tty**: a piped stdin *or* stdout gets a plain line read (there
  is nothing to repaint on a pipe); when stdin is still a terminal, the
  prompt goes to stderr, bash's rule, so the user sees where to type.
  Applies equally on Windows and Unix — raw-mode editing itself is not
  a narrowing on either platform (see "Platforms" below).

## Platforms

Full raw-mode editing — key decoding, the render engine, keymaps,
history, everything above — runs on both Unix and Windows 10+ (1809 or
later, for `ENABLE_VIRTUAL_TERMINAL_INPUT` support). The terminal
surface is backed by `rusty_libc`/`libc` (`termios`/`poll`/`read`/
`TIOCGWINSZ`) on Unix and by
[`rusty_win32`](https://github.com/baileyrd/rusty_win32)
(`GetConsoleMode`/`SetConsoleMode`/`ReadFile`/`WaitForSingleObject`/
`GetConsoleScreenBufferInfo`) on Windows — see `src/term_sys.rs` for the
shared interface both backends implement.

## Verification

Pure helpers (word motions in all three flavors, vi find targets, kill
ring append/prepend, yank-pop rotation, undo, case ops, word transpose,
last-arg cycling, prefix search, control-char visualization, CSI decode)
are unit-tested in `src/lib.rs`. End-to-end behavior — raw-mode escape
sequences, repaint math, bracketed paste, the full keymaps under a real
pseudo-terminal — is exercised downstream by rush's pty harness
(`tests/pty/editor_pty_test.py` in the rush repo, 28 scenarios), and by
this crate's own `tests/pty.rs` (30 scenarios) — both Unix-only; there
is no equivalent pseudo-terminal-driven behavioral suite for the
Windows backend, only compilation and the pure bit-math unit tests in
`term_sys.rs`'s Windows backend (`apply_raw_flags`/`is_raw`/
`clear_echo_flag`) plus whatever `rusty_win32`'s own tests already cover
at the primitive layer. Real interactive verification on Windows is
still outstanding.

## License

MIT.
