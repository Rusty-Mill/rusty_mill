# Changelog

## Unreleased

- Test coverage: a parity audit against `docs/survey.md`'s capability
  table found every documented capability genuinely implemented (no
  functional gaps), but 12 specific sub-behaviors named in that table had
  no test anywhere — `quoted_insert`'s literal control byte, the
  `C-x C-e`/`v` external-editor round-trip, `read_line_with_initial_timeout`
  as a combined seed+deadline call, the rustyline `#V2` history-header
  skip, `bell()` actually emitting `BellStyle::Audible`/`Visible` output,
  the `completion_query_items` y/n prompt and column-major candidate
  layout, plain (non-edge-case) abbreviation expansion, a multi-line
  bracketed paste end-to-end (live `⏎` render, single-unit insert, `; `
  -joined history recall), an unrecognized/private-mode CSI sequence being
  consumed whole instead of leaking into the buffer, raw-mode self-healing
  after an external cooked-mode change (approximating SIGTSTP+`fg` or a
  stray `stty`), `set_completion_ignore_case`/`set_show_all_if_ambiguous`
  actually changing behavior, and `terminal_size()`/`with_echo_disabled`
  (including panic-safety). All 12 now have unit or pty coverage; no
  production code changed — every one of the 12 was already correct.
  `docs/survey.md`'s "Deliberate narrowings" section also had one stale
  line (still claimed non-Unix builds get a buffered prompt-and-read,
  predating the Windows raw-mode backend) — fixed to match README.md's
  already-corrected copy.
- CI: dropped the `macos-latest` leg of the `test` job — GitHub Actions'
  macOS runners routinely took far longer to even start than every other
  job combined, for coverage `Feature (libc-backend)` already gives the
  same `libc`-crate code path (just on Linux, not real macOS/BSD — see
  `Cargo.toml`'s and the workflow's own comments on the gap this accepts).
  Real macOS/BSD behavior is unverified in CI now; verify by hand before
  a release if a change plausibly affects the `libc` backend specifically.
- New `prepare_external_output()`: lets a `Hooks::on_interrupted_read`
  implementation (already called on every ~200ms idle tick at the
  prompt, not just on a real `EINTR`) print its own output — a shell's
  background-job completion notice, say — without corrupting the
  editor's next repaint. Fully additive: no `Hooks` trait signature
  change, backed by a thread-local flag the existing self-heal repaint
  branch (previously only triggered by a clobbered raw mode or a
  resize) now also checks. New pty test
  (`prepare_external_output_prints_cleanly_while_idle_and_keeps_the_line`,
  via a new `examples/notify` demo) proves the notice lands on its own
  line and an in-progress buffer survives intact around it, the same
  shape `resize_while_idle_repaints_and_keeps_the_line` already proves
  for a resize.
- Windows raw-mode editing: `term_sys.rs` grows a `rusty_win32`-backed
  Windows implementation of its termios-shaped interface
  (`GetConsoleMode`/`SetConsoleMode`/`ReadFile`/`WaitForSingleObject`/
  `GetConsoleScreenBufferInfo` — not ConPTY, which hosts a *child*
  process's console rather than this process's own), and the raw
  editor internals (key decoding, render engine, keymaps, history,
  bracketed paste, `C-x C-e` edit-in-`$EDITOR`) — previously
  `#[cfg(unix)]`-only, falling back to a plain buffered read on
  Windows — now run on both platforms. No behavioral regression on
  Unix: the full existing test suite (`cargo test`, including the
  30-scenario `tests/pty.rs` pseudo-terminal suite) passes unchanged.
  The Windows path has no equivalent pseudo-terminal-driven test
  coverage yet (`tests/pty.rs` stays `#![cfg(unix)]`-only) — only
  compilation and `term_sys.rs`'s own pure bit-math unit tests are
  verified in CI; real interactive verification on Windows is still
  outstanding.

## 0.4.0 — 2026-07-17

- Ninth review pass: self-review of the previous performance pass, plus
  documentation and CI hygiene.
  - Fix: `render_owed` wasn't cleared after incremental search's own
    unconditional repaint (`SearchOutcome::Continue`/`Exit`) — entering
    search mid-coalesced-burst left the flag stale, causing one
    harmless but wasted extra render at the next `finish_line`. New pty
    test (`burst_into_search_paints_correctly`) covers entering and
    accepting a search within a single coalesced read.
  - Fix: repaint coalescing had no bound — a continuous input flood (a
    huge unbracketed paste, runaway automation) could show zero visual
    progress for as long as it lasted, indistinguishable from a hang.
    Capped at `MAX_COALESCED_RUN` (200): a paint is now forced
    periodically regardless of queued input. New pty test
    (`very_large_flood_completes_and_is_not_lost`) guards against
    regressions past the cap.
  - Docs: `term_sys::apply_raw_flags`'s doc comment said "one byte at a
    time," left over from before buffered reads — corrected to explain
    `VMIN`/`VTIME`'s actual (call-size-independent) semantics.
    `CLAUDE.md` and the crate-level doc header now describe repaint
    coalescing, which the previous pass added without updating either.
  - CI: a new `bench` job builds and clippy-lints `bench/` (which,
    being outside the workspace, was previously unverified by any CI
    job) — build/lint only, not a full run, to avoid pty-timing
    flakiness in CI.
- Performance: two fixes driven by `bench/`'s numbers, both internal
  (no API or behavior change; the pty test suite gained several tests
  targeting the exact hazard the second one had to avoid).
  - **Buffered stdin reads**: `read_byte` used to cost one `read()`
    syscall per byte (via `term_sys::read_stdin_byte`); it now pulls
    whatever the kernel has queued in one `read_stdin_chunk` call and
    serves individual bytes from an internal buffer, refilling only
    when it empties. `input_ready` consults the same buffer, so it can
    never disagree with `read_byte` about what's pending (the same
    hazard `io::Stdin`'s own opaque buffering caused historically — see
    `read_byte`'s doc comment). This fixed the benchmark's worst gap: a
    20 KB bracketed paste went from ~29ms to ~2ms, on par with
    rustyline and reedline (both were previously 10-15x faster here).
  - **Repaint coalescing**: the main loop now skips a repaint when more
    input is already queued (`input_ready(0)`, free when the stdin
    buffer above has bytes) — a skipped frame would be instantly
    overwritten by the next key's repaint anyway, readline's own
    trick. A new `render_owed` flag on `LineState` guarantees this is
    safe: `finish_line` (the single choke point every exit path —
    Enter, C-c, a completion listing, a host binding, C-x C-e — now
    funnels through) flushes any owed render *before* computing its
    cursor-repositioning math, which would otherwise corrupt the
    display if it ran against stale `painted_rows`/`painted_cursor_row`
    bookkeeping. This took burst-typing throughput from ~19µs/key to
    ~0.7µs/key and cut bytes written per keystroke at a long line from
    ~1100 to ~6 — both now favorable versus rustyline — with zero
    change to true per-keystroke (paced) latency, since normal
    interactive typing never has anything queued to coalesce.

- New `bench/` head-to-head benchmark harness: a standalone,
  non-workspace crate that spawns minimal rusty_lines, rustyline, and
  reedline REPLs under one pty driver (which answers cursor-position
  queries, required by reedline) and feeds identical byte streams —
  measuring paced per-keystroke latency, burst throughput, bytes
  painted per key, 20 KB paste ingestion, and history-arrow cost.
  Deliberately outside the workspace so the editor keeps its
  zero-dependency build; run with `cd bench && cargo run --release`.

- In-session history edits survive navigation: change a recalled entry,
  move away with Up/Down/M-</M->/prefix search/vi `G`, come back — the
  edit is still there (zsh's scope: everything reverts once a line is
  accepted, since the overlay lives per `read_line`). Previously every
  navigation re-cloned the original and the edit was lost.
- Leaving incremental search on a hit now *moves the history position*
  to the found entry, so a following Up walks backward from there
  (readline); the overlay for that entry is re-anchored to the matched
  original text.
- Abort semantics: C-c during incremental search aborts the whole read
  (bash's `^C` + fresh prompt) instead of merely leaving search mode;
  C-g/Esc abort the search with readline's bell; and C-g in normal
  editing is now readline's `abort` command (`EditorAction::Abort`,
  bell) instead of an unbound no-op — a host rebinding C-g (e.g. to a
  `bind -x` command) shadows it as before.
- The non-Unix fallback prints its prompt to stderr, matching the Unix
  plain path (bash's rule — the prompt is a conversation with the
  user, not output).
- `save_history` syncs the parent directory after the atomic rename
  (best-effort), so a crash immediately after saving can't resurrect
  the old file from an unsynced directory entry.
- New `read_line_with_initial_timeout`: the deadline and the pre-seeded
  buffer combined, so a `$TMOUT` host doesn't give up seeding.
- First end-to-end multibyte coverage: pty tests type CJK and accented
  text and edit through it (input assembly, double-width cursor math,
  word kills); a pty test pins the history-edit persistence; the chaos
  loop now feeds `visualize_marked` random mark offsets.
- Compile-time `Send`/`Sync` guarantees on the public types (`Editor`,
  `ReadResult`, `Candidate`, `EditorAction`, `NoHooks`) — hosts move
  the editor across threads; a refactor can't silently break that.
- The remaining `Vec`-collecting text helpers (`vi_word_back`,
  `vi_word_end`, `transpose_words`) became iterator walks — no more
  per-keystroke line copies in vi mode and M-t.
- README: the intro no longer claims the crate always uses `libc` —
  Linux defaults to the zero-third-party-code `rusty_libc` backend
  (true since 0.2.0); the try-it line lists the grown example set.

## 0.3.0 — 2026-07-17

- Fix: EOF on stdin with text pending returned `Eof` and discarded the
  line in the raw path; it now returns the line (readline's rule, and
  what the piped-stdin path already did) — only an empty line is
  end-of-input.
- Fix: the output path used `print!`/`println!`, which panic on write
  failure — a terminal vanishing mid-session (ssh drop, closed
  emulator) killed the host with "failed printing to stdout" instead
  of surfacing an `io::Error` like the read side. The whole paint path
  is now fallible; the RAII guards, which cannot propagate, ignore
  write errors.
- The plain fallback prints the prompt to *stderr* when stdin is a
  terminal (bash's rule): with stdout piped (`host | tee`), the
  interactive user got no prompt at all after the stdout-tty gate.
- Undo now coalesces runs of single-character deletes (Backspace /
  Delete) exactly like insert runs, sharing the 20-character cap —
  undoing a long rubout was one undo per character (readline groups
  both).
- Incremental-search Backspace is position-disciplined like typed
  characters: shrinking the query re-searches at-or-before the current
  hit instead of teleporting to the newest match.
- `load_history` under `erasedups` is now O(n): one backward pass with
  a hash set (last occurrence wins — provably what sequential adds
  produce) replaces the per-line linear scan, which made loading a
  large history file quadratic.
- End-to-end coverage for the right-side prompt: new
  `examples/rprompt.rs` and a pty test asserting the rprompt paints
  and hides once the line grows into it (the zsh behavior the matrix
  advertises).
- Key-spec property test: every key the decoder can produce (a sweep
  of all single bytes, ESC pairs, and CSI/SS3 finals) round-trips
  through `key_spec` → `parse_key_spec`; `common_prefix` joined the
  byte-soup chaos loop.
- API polish: `EditorAction` implements `Display` (the readline
  command name, so `bind -P`-style output is `format!("{action}")`)
  and `NoHooks` derives `Debug`/`Clone`/`Copy`/`Default`.
- Release housekeeping: version 0.3.0 cut per `docs/RELEASING.md` —
  this section collects everything merged since 0.2.0 (the review
  passes #16–#21), including the `Hooks::highlight` contract change
  noted below. Minor-version bump per the pre-1.0 breaking-change
  rule. MSRV unchanged at 1.88.

- Fix (raw-mode recipe): `ISTRIP` and `IGNCR` are now cleared on entry
  to raw mode. If the inherited termios had `ISTRIP` set, every UTF-8
  high byte lost its top bit (all non-ASCII input corrupted); with
  `IGNCR` set, the `\r` that Enter sends was discarded — Enter was
  dead. readline clears both.
- Fix: `poll_stdin` treated EINTR (and any poll error) as "no input",
  so a signal landing during the short polls misdecoded ESC as a lone
  Esc, abandoned half-read escape sequences, and made bounded waits
  give up early. Both backends now retry on EINTR.
- Self-healing raw mode: the 200ms idle tick verifies the terminal
  still matches the raw recipe and re-asserts it (plus a repaint on a
  fresh line) when an external SIGTSTP/`fg` or a host command's `stty`
  left the tty cooked — readline's SIGCONT re-preparation, without
  installing a signal handler (which stays the host's business).
- Raw editing now also requires *stdout* to be a terminal: with stdout
  piped (`host | tee log`) the editor used to spray repaint escape
  sequences into the pipe; it now falls back to the plain read, same
  as for a piped stdin.
- Fix: rapid double-Esc (a vi user mashing into normal mode) was
  swallowed whole — ESC followed by ESC within the 30ms window decoded
  to nothing. It now decodes as Esc.
- History round-trip: entries that *look like* timestamp comments are
  no longer eaten by `load_history`. Interactive shells store comment
  lines, so `#42` is a legitimate entry; now only epoch-scale stamps
  (nine or more digits) parse as timestamps, a stamp always pairs with
  the following entry line (even a stamp-shaped one), and a dangling
  stamp is kept as an entry. A lone epoch-scale comment followed by
  another entry remains inherently ambiguous in bash's format
  (documented on `load_history`).
- CI: every job now builds `--locked`, so the committed `Cargo.lock`
  is authoritative and a push to the `rusty_libc` git dependency's
  tracked branch can no longer change or break CI builds silently.
- Chaos tests: a seeded, deterministic byte-soup test hammers the pure
  decoders and text helpers (`parse_key_spec`, `decode_key_bytes`,
  `csi_key`, `visualize*`, `display_width`, the word/object/find
  helpers, `clamp_start`) asserting nothing panics — always-on cheap
  fuzzing with no cargo-fuzz dependency.
- Render: the buffer is now visualized once per keystroke instead of
  twice — a single marked pass measures the paint, the cursor column,
  and the total width together (`visualize_marked`).
- API polish: `ReadResult` derives `Debug`/`Clone`/`PartialEq`/`Eq`
  (hosts can `assert_eq!` on it), `Candidate` derives `Debug`/`Clone`,
  and `Editor` derives `Debug`.
- Pre-seeded lines: `Editor::read_line_with_initial(prompt, rprompt,
  hooks, (left, right))` starts the edit with text in the buffer and
  the cursor between the halves (rustyline's `readline_with_initial`,
  zsh `print -z`) — `fc`-style edit-and-rerun, offered corrections.
  Ignored on a non-tty stdin and on the non-Unix fallback.
- Command-name mapping: `EditorAction::name()` and
  `EditorAction::from_name()` expose readline's command names
  (`kill-line`, `menu-complete-backward`, …) so a host's `bind` builtin
  doesn't maintain its own drift-prone table; the exhaustive match
  forces future actions to get names.
- Host-binding introspection: `Editor::host_bindings()` lists the
  `bind_host` entries as (key spec, tag) pairs — bash's `bind -X`,
  which `bindings()` deliberately omits.
- Fix: `operate-and-get-next` (C-o) recalled the wrong entry when
  `erasedups` (or a host history edit) shifted indices between lines;
  the recall now stores the entry text and re-locates it by content.
- Fix: pasting during incremental search exited the search and
  discarded the paste; it now types into the query (bash's behavior),
  with newlines flattened to spaces.
- Incremental search starts from the current history position after an
  unedited Up-recall (readline continues backward from point in
  history) instead of restarting at the newest entry; typed characters
  search at-or-before the current match. An edited recall does not
  seed the position, so exiting an empty search can never clobber
  edits.
- Hardening: out-of-range or mid-character word-start offsets returned
  by `Hooks::complete` / `Hooks::expand_abbreviation` are clamped to a
  char boundary at or before the cursor instead of panicking the read
  loop — a hook bug must not take the host shell down.
- Fix: the piped-stdin path (`read_line_plain`) swallowed EINTR without
  calling `Hooks::on_interrupted_read`, so hosts running piped scripts
  never got their trap callback; it now fires like the raw path.
- Fix: Shift-Tab was dead in vi normal mode (it tore down the armed
  completion menu and did nothing); it now reverse-cycles there, like
  Tab completes there.
- Performance: the alphanumeric word motions no longer collect the
  line into a `Vec` per keystroke, and case-insensitive search matching
  compares case-folded char streams instead of allocating two lowercase
  `String`s per history entry per keystroke.
- Incremental search now leaves the cursor *on* the matched text when
  the search is accepted or exited (readline's point), instead of at
  end-of-line — you searched for it to edit it.
- Prefix search (PageUp/PageDown) re-anchors on the buffer up to the
  cursor whenever the previous key wasn't itself a prefix search (zsh's
  rule). Previously the anchor was captured only when off history, so a
  PageUp after a plain Up searched with a stale or empty prefix.
- Undo now chunks runs of self-insert into groups of at most 20
  characters (readline's grouping): one C-_ no longer wipes an entire
  typed line.
- Completion append character (readline's
  `rl_completion_append_character`): a unique, fully-inserted match is
  followed by a space so the cursor is ready for the next word;
  `set_completion_append_character` changes or disables it.
- `possible-completions` (M-?): list the candidates without touching
  the buffer; `insert-completions` (M-*): insert every match,
  space-separated — both stock readline commands.
- `character-search` (C-]): reads one character and moves the cursor to
  its next occurrence; `character-search-backward` is available unbound
  (its readline default M-C-] isn't a decodable chord here).
- vi: the Delete key now kills into the ring like `x` (vim), honoring
  counts, instead of discarding the text.
- Alt + non-ASCII chords (`\M-ö`) decode: the ESC-prefixed UTF-8
  sequence is assembled whole — previously the lead byte was consumed
  and the continuations decoded as garbage keys.
- Tabs render at 8-column stops of the true display offset (prompt
  included) instead of a fixed four spaces, so tab-indented pastes line
  up the way terminals show them; cursor math tracks the same expansion.
- `BellStyle::Visible` holds the reverse-video flash ~80ms before
  clearing it (input ends it early) — set-then-unset in a single write
  could render zero frames on many terminals.
- Multi-line prompts (a `PS1` with newlines) now work: everything up to
  the prompt's last newline paints once per edit region (readline's
  approach); only the final line joins the per-keystroke repaint. The
  old row accounting duplicated the prompt down the screen on every
  keystroke.
- `Hooks::highlight` contract change: the hook now receives the *raw*
  buffer (exactly what Enter returns — real tabs/newlines, true byte
  offsets) instead of the control-char-visualized text, so a parser
  highlights what the user actually typed. The hook's SGR markup passes
  through and the editor re-applies the `^X`/`⏎`/tab visualization
  around it; a non-SGR escape from the hook is neutralized rather than
  sent to the terminal, and a buffer containing a literal ESC paints
  unhighlighted (the markup would be ambiguous). Hosts that compensated
  for the visualized input should drop that compensation.
- Flicker-free repaint: the render used to clear the edit region and
  then repaint it, showing a blank frame every keystroke (visible on
  slow terminals and ssh). It now overwrites in place and erases only
  the leftover tail (paint-then-clear, readline's redisplay order).
- History file safety: `save_history` writes atomically (sibling temp
  file + rename, so a crash mid-write can't truncate the history) and
  new files are created mode 0600 on Unix, like bash's history file —
  history routinely contains secrets. Existing files keep their
  permissions; `append_history`'s create path is 0600 too.
- Case-insensitive history search: `Editor::set_search_ignore_case`
  (readline 8.1's `search-ignore-case`) covers C-r/C-s incremental
  search and the PageUp/PageDown prefix search.
- Reverse menu cycling: Shift-Tab (`CSI Z`, decoded with or without
  modifier parameters) is `EditorAction::MenuCompleteBackward` —
  readline's `menu-complete-backward`, zsh's `reverse-menu-complete`;
  a cold backward step starts on the last candidate.
- vi counts for `r`, `p`, `P` (vim semantics): `3rx` replaces three
  characters — failing outright, with a bell, when fewer remain — and
  `3p`/`3P` paste three copies.
- OSC-aware width math: `display_width` now skips OSC sequences
  (BEL- or ST-terminated) — a prompt carrying an OSC-8 hyperlink or a
  window-title sequence had its whole payload counted as printable
  width, misplacing the cursor.
- Fix: an unrecognized CSI sequence was cut at its first non-digit
  byte, so an SGR mouse report (`ESC[<65;5;10M`) leaked `65;5;10M`
  into the buffer as typed text. The decoder now consumes the full
  ECMA-48 grammar (parameter bytes 0x30–0x3F, intermediates 0x20–0x2F,
  one final byte) and swallows unknown sequences whole.
- New `examples/vi.rs` (vi mode + mode indicator + a deliberately
  multi-line prompt) and pty tests for normal-mode editing, `daw`,
  count-replace, and once-per-region prompt-prefix painting.
- Operate-and-get-next (readline, bash C-o): `EditorAction::OperateAndGetNext`
  accepts the line and pre-loads the next `read_line` with the history
  entry after the one just executed, for replaying command sequences.
- New named actions: `KillWholeLine` (unbound by default, like readline)
  and `DeleteHorizontalSpace` (M-\; a delete, not a kill — readline's
  spaces-and-tabs-around-point rule).
- vi mode additions (vim semantics; readline `vi-fetch-history` for `G`):
  `%` matching-bracket motion (inclusive under operators, both
  directions), `G` fetches the most recent history entry — or entry N
  with a count — and the `iw`/`aw` word text objects for `d`/`c`/`y`
  (`diw`, `caw`, `yiw` …).
- Mode indicator: `Editor::set_show_mode_in_prompt` prefixes readline's
  default mode strings — `(ins)`/`(cmd)` in vi mode, `@` in emacs mode —
  to the prompt (readline `show-mode-in-prompt`).
- Completion listing matches readline more closely: candidates are
  sorted and laid out column-major, and a list of
  `completion-query-items` or more (default 100; 0 disables;
  `set_completion_query_items`) asks `Display all N possibilities?
  (y or n)` before printing.
- Incremental search failure feedback: a query with no match shows
  readline's `(failed reverse-i-search)` label, keeps the last match
  visible, and rings the bell. The bell (still governed by
  `set_bell_style`) now also rings on history motion past either end,
  a failed prefix search, and a failed vi `f F t T ; ,` find.
- History ignore-space option: `Editor::set_history_ignore_space` makes
  `add_history_entry` skip lines starting with a space (bash
  `HISTCONTROL=ignorespace`).
- Kill-ring and undo depths are configurable (`set_max_kill_ring_len`,
  `set_max_undo_len`; defaults unchanged at 32/200), and both now evict
  in O(1) (`VecDeque`) instead of shifting the whole buffer per
  keystroke once full.
- `Hooks::hint` is now called at most once per keystroke (memoized on
  the buffer content) instead of up to twice — render and the
  Right/End/M-f accept paths share the cached value.
- Fix: a bracketed paste in vi *normal* mode was silently discarded;
  it now inserts literally at the cursor (vim, readline vi mode).
- Fix: Ctrl-Space and Ctrl-\ / Ctrl-] / Ctrl-^ self-inserted raw
  NUL/FS/GS/RS bytes into the buffer; they now decode as `\C-@`,
  `\C-\\`, `\C-]`, `\C-^` — unbound by default but rebindable — and
  `key_spec` escapes backslashes so such bindings round-trip.
- Fix: the C-x C-e scratch file had a predictable name in a shared
  `$TMPDIR` (symlink-attack window) and default permissions; it is now
  created `O_EXCL` with an unpredictable name and mode 0600, and the
  path is shell-quoted so a `$TMPDIR` with spaces works.
- Fix: a stray UTF-8 continuation byte swallowed the next three
  keystrokes as "continuations"; invalid lead bytes now become U+FFFD
  immediately, and continuation reads are time-bounded.
- Fix: the `read_line_timeout` deadline is now honored inside C-x
  chords and quoted-insert (which block on their own follow-up key),
  and half-delivered escape sequences / unterminated pastes give up
  after a bounded wait instead of hanging the read.
- Fix: the piped-stdin fallback (`read_line_plain`) left a trailing
  `\r` on CRLF input; it now strips it, matching the non-Unix fallback.
- Key rebinding API (revisiting a narrowing; rush's `bind` builtin):
  the emacs/vi-insert commands are now the public `EditorAction` enum,
  and `Editor::bind`/`unbind`/`bindings` remap single keys using
  readline's key-spec spellings (`\C-x`, `\M-f`, `\e[1;5C`, backslash
  escapes). `unbind` masks a default (readline `bind -r`); `bindings()`
  lists the effective keymap (`bind -P`). Multi-key chords, vi normal
  mode, and `.inputrc` parsing stay out of scope.
- Host-command bindings — bash's `bind -x`: `Editor::bind_host` tags a
  key; pressing it suspends raw mode and calls the new
  `Hooks::host_binding(tag, &mut line, &mut cursor)`
  (`READLINE_LINE`/`READLINE_POINT` contract), then repaints.
- Readline variables: `set_completion_ignore_case`,
  `set_show_all_if_ambiguous`, `set_menu_complete` (Tab becomes
  readline's `menu-complete`), and `set_bell_style` (`BellStyle`;
  audible by default like readline — the editor now rings on completion
  with no candidates, so hosts wanting the old silence set
  `BellStyle::None`).
- Read deadline (bash `$TMOUT`): `Editor::read_line_timeout` and the
  new `ReadResult::TimedOut` variant. The deadline is measured from the
  call and checked on the idle poll tick and between keystrokes; hosts
  matching on `ReadResult` exhaustively need a new arm.
- History timestamps (bash `HISTTIMEFORMAT` file format): entries are
  stamped on add; `load_history` parses `#<epoch>` comment lines (plain
  files still load, both formats round-trip); `save_history`/
  `append_history` emit them only under `set_history_timestamps(true)`,
  so existing plain files are not rewritten unasked;
  `history_timestamps()` exposes the stamps for the host's `history`
  builtin.
- In-place history replacement: `Editor::replace_history` resyncs the
  editor's list after a host's `history -c`/`-d` without rebuilding the
  editor (kill ring and session state survive); replaced entries count
  as persisted, so `append_history` stays incremental.
- Terminal facilities: public `terminal_size()` — (cols, rows) from
  stdout, for `$COLUMNS`/`$LINES` (bash `checkwinsize`) — and
  `with_echo_disabled(f)` — panic-safe echo-off around a closure, the
  termios replacement for shelling out to `stty` in `read -s`.
- New `examples/timeout.rs`; pty tests for the timeout, a rebound key,
  and a host binding; `examples/hooked.rs` grew a rebinding and a host
  binding to drive them.
- Fix: the non-Unix `read_line_timeout` fallback printed the prompt
  unconditionally, unlike the Unix path (which suppresses it for a
  non-tty stdin, falling back to `read_line_plain`). A script piped into
  an "interactive" host on Windows — rush's own `-i` test harness, for
  one — got prompt text mixed into captured stdout. Now gated on
  `IsTerminal::is_terminal`, matching the Unix behavior.

## 0.2.0 — 2026-07-11

- The terminal syscall surface (termios/poll/read/winsize) moved into
  `src/term_sys.rs` with two backends: the hand-rolled `rusty_libc`
  raw-syscall crate — now the default on Linux, linking no third-party
  libc bindings — and the `libc` crate (other Unix; on Linux via
  `--no-default-features --features libc-backend`). CI exercises both.
- Releases are git-based (tag + GitHub release), not published to
  crates.io: `publish = false` in Cargo.toml, and `docs/RELEASING.md`
  reworked accordingly (the crates.io section replaced by a downstream
  rush harness check).
- Menu cycling: Tab after the candidate list walks the candidates
  in-line, wrapping around (zsh `AUTO_MENU`, readline `menu-complete`) —
  revisiting a narrowing; the paging/menu-select UI stays declined.
- Resize while idle at the prompt now repaints within a poll tick
  (~200ms), without a SIGWINCH handler. The idle tick also calls
  `Hooks::on_interrupted_read`, so pending host traps fire promptly even
  with no input arriving.
- History dedup option: `Editor::set_history_dedup` erases earlier
  duplicates on add (bash `HISTCONTROL=erasedups`, fish). Off by default.
- Revert line: M-r undoes every edit to the line at once (readline
  `revert-line`).
- New `examples/hooked.rs` (completion + hints demo) used by new pty
  tests for menu cycling, hint acceptance, and idle-resize repaint.
- `docs/RELEASING.md`: release checklist (crates.io blockers, MSRV sync
  points, tag/publish steps).
- History cap: `Editor::set_max_history_len` (readline `stifle_history`,
  bash `HISTSIZE`); oldest entries drop past the limit.
- Append-only history persistence: `Editor::append_history` writes only
  entries added since the last load/save/append (bash `histappend`), so
  concurrent sessions interleave instead of overwriting. `save_history`
  now takes `&mut self` to track what's persisted.
- Partial hint acceptance: M-f / Ctrl-Right at end of line accepts one
  word of the history hint (fish's forward-word on an autosuggestion).
- Documented existing C-l clear-screen in the feature matrix; recorded
  positions on eager resize repaint and grapheme-cluster width math as
  deliberate narrowings.
- `docs/survey.md`: the per-editor line-editor field survey behind the
  README's feature matrix (reconstructed).
- End-to-end pty test suite (`tests/pty.rs`): drives `examples/demo` under
  a pseudo-terminal — typing/echo, emacs editing, kill/yank, history
  recall, Ctrl-C, and bracketed paste.
- CI (build/test on Linux, macOS, Windows; clippy, rustfmt, docs, MSRV),
  dependabot, `rust-version = "1.88"`, API docs for all public items with
  `#![warn(missing_docs)]`, and a compiled crate-level usage example.

## 0.1.0

- Initial import: the line editor extracted from the
  [rush shell](https://github.com/baileyrd/rush). Emacs + vi keymaps,
  kill ring, undo, incremental and prefix history search, bracketed
  paste, completion/hint/highlight/abbreviation hooks, right-side
  prompt, wide-char and ANSI-aware rendering.
