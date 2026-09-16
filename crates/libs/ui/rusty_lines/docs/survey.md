# The line-editor field survey

The audit behind `rusty_lines`: every line editor in wide use was
examined, its behaviors catalogued, and each capability either matched
(with a named reference implementation) or consciously declined (with a
reason). The README carries the distilled matrix; this document is the
fuller per-editor record it was distilled from, reconstructed after the
extraction from rush.

Two questions drove the audit, in both directions:

1. **Coverage** — is there a capability users of *any* mainstream editor
   would reach for and miss here?
2. **Fidelity** — where a capability exists, does it match the semantics
   of the editor that defined it (not a lookalike approximation)?

## The field

### GNU readline

The baseline against which everything else defines itself; bash, gdb,
psql, and hundreds of others link it.

- **Taken as the reference for:** the emacs keymap core (C-a/C-e/C-b/C-f,
  C-d, C-h, C-t, M-t, M-u/M-l/M-c); the kill ring with append/prepend on
  consecutive kills and M-y yank-pop rotation; the *two word classes*
  (M-b/M-f/M-d over alphanumeric words vs `unix-word-rubout` C-w over
  whitespace words — a distinction most clones flatten); undo with
  self-insert runs coalesced (and, notably, *no redo* — readline itself
  has none); C-r/C-s incremental search with mid-search direction
  switching; M-. / M-_ insert-last-argument with repeat cycling; C-v/C-q
  quoted insert with `^X`-style control-char rendering; C-x C-e
  edit-in-`$EDITOR`; bracketed paste (default-on since readline 8.1);
  the vi keymap tradition (below, shared with ksh); list-style completion
  (`CompletionType::List`: longest-common-prefix insertion, then a
  columned candidate list); C-l clear-screen; the history cap
  (`stifle_history` / bash `HISTSIZE`), bash's `histappend`
  append-only persistence and `HISTCONTROL=erasedups` dedup (opt-in),
  and `revert-line` (M-r); single-key rebinding (`bind '"\C-x": function'`,
  `bind -P`, `bind -r`) over a named-action enum, bash's `bind -x`
  host-command contract (`READLINE_LINE`/`READLINE_POINT`), the
  `completion-ignore-case` / `show-all-if-ambiguous` / `menu-complete` /
  `bell-style` / `completion-query-items` / `show-mode-in-prompt`
  variables, the sorted column-major completion listing,
  `operate-and-get-next` (C-o), `delete-horizontal-space` (M-\) and
  `kill-whole-line`, the `(failed reverse-i-search)` state, readline
  8.1's `search-ignore-case`, `menu-complete-backward` (Shift-Tab),
  multi-line prompt handling (prefix lines print once, the final line
  repaints), the 0600 history file, bash's `HISTCONTROL=ignorespace`,
  `possible-completions` (M-?) and `insert-completions` (M-*),
  `character-search` (C-]), the completion append character, the
  20-character self-insert undo chunking, point landing on the match
  when a search ends, the search continuing from the current history
  position, the command-name mapping (`EditorAction::name`/`from_name`,
  readline's `rl_named_function`), bash's `bind -X` host-binding
  listing (`host_bindings()`), and the read timeout (`rl_set_timeout`,
  bash `$TMOUT`).
- **Declined:** `.inputrc` *file parsing* (the host's `bind` builtin
  passes readline key specs through instead); multi-key chord bindings
  beyond the built-in C-x set; keyboard macros (C-x `(` … C-x `)`);
  emacs-mode numeric arguments (M-digit, C-u); mark/region (C-@,
  C-x C-x). (`menu-complete` and single-key rebinding were initially
  declined, then adopted — see the matrix.)

### libedit (editline)

BSD's independently licensed readline-alike (`.editrc`), shipped where
GPL is unwanted (macOS system tools, LLDB). Functionally a subset of
readline — surveying it added no capability readline didn't already
demand, but it confirms which subset of the emacs keymap is the
portable, load-bearing core.

### zsh ZLE

The most featureful shell editor; everything is a widget bound in
`emacs`/`viins`/`vicmd` keymaps.

- **Taken as the reference for:** the right-side prompt (`$RPS1`),
  including hiding it once the line grows into it; prefix history search
  (`history-beginning-search-backward/-forward`, here on
  PageUp/PageDown and M-p/M-n); forward incremental search being a
  first-class peer of backward (which requires IXON off so C-s reaches
  the editor); repeated-Tab candidate cycling after the list (`AUTO_MENU`,
  without the highlighted menu-select UI); case ops and
  insert-last-argument parity with readline.
- **Declined:** user-defined widgets and rebindable keymaps; the
  editable multi-line buffer (ZLE moves the cursor across embedded
  newlines); menu-select completion; ZLE's undo *and redo* pair (redo
  declined to stay with readline semantics).

### fish

The editor that made "helpful by default" the expectation.

- **Taken as the reference for:** history hints / autosuggestions (the
  dimmed inline continuation, accepted with Right/End, or one word at a
  time with M-f / Ctrl-Right, fish's forward-word on a suggestion) —
  surfaced here through `Hooks::hint`; syntax highlighting while typing —
  `Hooks::highlight`; abbreviation expansion on space (`abbr`) —
  `Hooks::expand_abbreviation`; C-z as an undo binding alongside
  readline's C-_ / C-x C-u; Up-arrow prefix search behavior folded into
  the prefix-search design.
- **Declined:** the completion pager with interactive selection; true
  multi-line buffer editing (fish edits command blocks in-buffer);
  fish's `bind` programmability.

### ksh93 (emacs + vi modes)

Where shell vi mode actually comes from; readline's vi keymap follows
it. Its emacs mode is also the historical source of several readline
defaults.

- **Taken as the reference for:** vi-mode structure and quirks shared
  with readline vi mode: counts; `d`/`c`/`y` operators over motions;
  `h l 0 ^ $ w W b B e E f F t T ; ,`; `x X D C s S Y r ~ p P u`;
  `i I a A`; `k`/`j` for history; the `cw` ≡ `ce` quirk (change-word
  stops at word end, vim-style, not vi-motion-style); Esc backing the
  cursor up one column on exit from insert. Vim (the tradition's living
  reference) supplies `%` bracket matching, `G` history fetch by count
  (readline `vi-fetch-history`), and the `iw`/`aw` word text objects.
- **Declined:** nothing beyond what the vi narrowings below already
  cover — ksh's vi mode is close to the floor of the tradition.

### linenoise

The minimalist pole (~1k lines, redis/valkey): proof of how small an
editor can be. Surveyed to calibrate the floor, and for one idea that
generalized well — the **hints callback**, which linenoise pioneered in
C and fish popularized as autosuggestions; `Hooks::hint` descends from
both. Its single-line-repaint model also informed the render engine's
"repaint the whole edit region each keystroke" approach (extended here
with soft-wrap row accounting linenoise lacks).

### replxx

The C++ continuation of the linenoise lineage (via linenoise-ng), used
by ClickHouse and others. Confirms the "modern minimal" feature floor:
UTF-8, syntax highlighting, hints, completion, incremental search.
Highlighting-while-typing as a callback (rather than a plugin system, as
in ZLE) matches the `Hooks::highlight` shape.

### rustyline

The Rust readline port this editor replaced inside rush — the survey's
most direct comparison.

- **Matched for continuity:** the host-integration shape (rustyline's
  `Helper` trait ≈ `Hooks`: completer, hinter, highlighter); emacs + vi
  keymaps; kill ring; undo; `readline_with_initial`
  (`read_line_with_initial` here); and one deliberate migration
  affordance — `load_history` skips rustyline `FileHistory`'s `#V2`
  header so an existing history file keeps working after the swap.
- **Declined:** the validator hook (multi-line continuation decisions
  belong to the host's parser, not the editor); configurable
  completion types beyond list-style.

### reedline

Nushell's editor; the other prominent Rust implementation.

- **Confirms:** right prompt, hinter/highlighter hooks, bracketed
  paste, vi mode — the same modern baseline this editor targets.
- **Declined:** menu system (completion menus, history menu);
  multi-line buffer editing; its keybinding configuration layer.

### prompt_toolkit / PSReadLine

The out-of-shell poles — Python's REPL toolkit (ptpython, IPython) and
PowerShell's editor.

- **Taken as the reference for:** prefix history search on arrows
  (PSReadLine's `HistorySearchBackward` matches the PageUp behavior
  here); predictions/autosuggestions (PSReadLine `Prediction`,
  prompt_toolkit `auto_suggest`) reinforcing the hint design.
- **Declined:** full-screen application features (prompt_toolkit is a
  TUI toolkit that happens to include a prompt); PSReadLine's
  undo/redo pair; completion menus in both.

## Capability audit

Every row names the editor whose semantics are matched — fidelity is to
that implementation, not a generic average. (This table also lives in
the README; kept in both places deliberately.)

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

- **Multi-line *buffer editing*** (zsh/fish/reedline/prompt_toolkit edit
  a `\n`-separated buffer with per-line cursor movement). The buffer is
  one logical line; embedded newlines (from a paste or C-v C-j) render
  as `⏎` and return correctly, but Up/Down navigate history, not buffer
  rows. C-x C-e hands real multi-line editing to `$EDITOR`.
- **Full keymap programmability** (readline's `.inputrc`, ZLE widgets,
  fish's `bind` functions, reedline's keybinding config). Single-key
  rebinding of the named actions and host-command bindings *are*
  supported (see the matrix — revisiting a narrowing); what stays
  declined: user-defined widgets (actions are the built-in enum, not
  host code — except `bind_host`, which is exactly bash's `bind -x`
  scope), multi-key chord bindings beyond the built-in C-x set,
  rebinding vi normal mode, and `.inputrc` file parsing (the host's
  `bind` builtin passes readline key specs through verbatim).
- **Keyboard macros** (readline C-x `(` … `)`), **numeric arguments in
  emacs mode** (M-digit; vi counts *are* supported), **mark/region**
  (C-@, C-x C-x), **redo** (ZLE, fish, and PSReadLine have it; readline
  has none either — readline semantics win).
- **vi registers, `.` repeat, `/` history search** (C-r covers search
  from insert mode; the unnamed register is the kill ring).
- **Completion paging and menu-select UI** (fish's pager, ZLE's
  interactive menu with a highlighted selection, reedline/prompt_toolkit
  menus): long candidate lists print unpaged — though readline's
  `completion-query-items` y/n guard (see the matrix) asks before
  dumping a big one; repeated-Tab cycling stands in for menu-select.
- **Signal-driven resize repaint** (readline installs a SIGWINCH
  handler). The width is re-read from the tty on every repaint, and a
  resize while idle at the prompt is noticed by the input poll tick and
  repainted; only the signal handler itself is declined — installing
  one from a library is the host's business, not the editor's, and the
  host already owns signal delivery via `Hooks::on_interrupted_read`.
- **Grapheme-cluster cursor math** (combining marks, emoji ZWJ
  sequences). Width is per-`char` via `unicode-width`; getting clusters
  right would add a `unicode-segmentation` dependency against the
  crate's two-dependency budget, while terminals themselves disagree on
  cluster widths — the common-case behavior (wide CJK, zero-width
  combining marks) is what `unicode-width` already gives.
- **Non-tty**: a piped stdin *or* stdout gets a plain line read (there
  is nothing to repaint on a pipe); when stdin is still a terminal, the
  prompt goes to stderr, bash's rule, so the user sees where to type.
  Applies equally on Windows and Unix — raw-mode editing itself is not
  a narrowing on either platform (see the README's "Platforms"
  section).

## Maintaining this document

When adding a capability, name the reference editor and match *its*
semantics, then add the row to both this table and the README's. When
declining one, record it under narrowings with the reason — the
narrowings list is the part of the audit that keeps future feature
creep honest.
