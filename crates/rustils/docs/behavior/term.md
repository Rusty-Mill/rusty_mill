# Behavior: Terminal (RFC v2 §5; extraction map D9)

The parity suite asserts what CI can observe (redirected streams); the
interactive half is proved by `rterm` by hand and by the mock's
contract tests.

## is_tty

- Never errors: a stream that cannot be probed is not a tty.
- Linux: `isatty` (a `tcgetattr` probe). Windows: `GetConsoleMode`
  succeeding on the std handle — a redirected pipe/file handle fails it.
- **Parity-pinned (both legs):** with all three streams redirected (the
  CI shape), every stream reports `false`.

## window_size

- The controlling terminal's size in character cells, probed from the
  first tty among stdout, stderr, stdin.
- Windows reports the **viewport** (`srWindow`), not the scrollback
  buffer — the donor distinction (rusty_win32) is contractual here.
- **Parity-pinned (both legs):** with no terminal attached it returns
  `Err` (never a made-up size, never a panic). Fallback width policy
  (e.g. 80) belongs to consumers.

## enter_raw / leave_raw

- Raw mode: no echo, no line buffering, no signal-generating keys.
  Linux: `cfmakeraw` recipe over termios, applied with `TCSADRAIN`.
  Windows: clear `ECHO|LINE|PROCESSED` input bits, set
  `ENABLE_VIRTUAL_TERMINAL_INPUT` (+ VT processing on stdout when it is
  a console) — the Win10+ console then speaks the same raw byte dialect
  as a Unix tty.
- `enter_raw` saves prior state; `leave_raw` restores exactly it. Both
  idempotent: re-enter is a no-op, leave without enter is a no-op.
- **Parity-pinned (both legs):** with stdin redirected, `enter_raw`
  returns `Err` and a subsequent `leave_raw` is an `Ok` no-op.
- Restore-on-error discipline is consumer-owned; the reference shape is
  `coreutils::term_report::with_raw` (restores on success *and* on body
  error — mock-tested).

## is_raw

- A **live** probe — re-queried from the OS on every call, never cached
  from the last `enter_raw`. This is the point of the primitive: it is
  what lets a consumer (rusty_lines' self-healing idle tick) notice
  drift from something outside this handle — an external `stty`
  invocation, a suspended-then-foregrounded shell — that changed the
  terminal's mode without going through `enter_raw`/`leave_raw` at all.
- Linux: `tcgetattr` then check `ICANON`/`ECHO` both clear. Windows:
  `GetConsoleMode` then check `ENABLE_LINE_INPUT`/`ENABLE_ECHO_INPUT`
  both clear. A handle that cannot be queried reports `false` — a
  best-effort probe, not a fallible operation.
- **Parity-pinned (both legs):** with stdin redirected, `is_raw()` is
  `false` (matching `enter_raw`'s refusal on the same stream).

## poll_readable / read_chunk

- `poll_readable` blocks on stdin up to a timeout (`None` = forever),
  returning whether it became readable — works on any stdin, tty or
  not (a redirected pipe polls fine). Linux: `poll(2)`. Windows:
  `WaitForSingleObject` on the console input handle — coarser than
  "a byte is ready" (any input record wakes it, not just keystrokes),
  but `ReadFile` afterward blocks correctly on whatever was actually
  queued, so a spurious wake costs one extra round trip, never a wrong
  read (recorded here rather than as a divergence: both ends produce
  the same observable contract, only the wake granularity differs).
- `read_chunk` is one call, batched — not per-byte. In raw mode this is
  the VMIN=1/VTIME=0 shape: blocks for at least one byte, then returns
  whatever else is already buffered without a second round trip.
  `Ok(0)` is EOF, matching `std::io::Read`'s convention.
- **Live-verified** (not parity-pinned — timing-sensitive, exercised by
  hand): `rterm --raw-probe` under a real pty polls, then reads a
  batched chunk, entirely through these two primitives.

## set_echo

- Toggles local echo on stdin independent of full raw mode — the
  password-prompt shape: canonical/line-editing mode stays on, only
  the terminal's echo of typed characters is suppressed. Linux: flip
  `ECHO` in `c_lflag`, `tcsetattr(TCSADRAIN)`. Windows: flip
  `ENABLE_ECHO_INPUT`, `SetConsoleMode`.
- Returns the *previous* on/off state, so a caller restores exactly by
  calling `set_echo(previous)` — the raw-mode save/restore discipline,
  scaled to the one bool this operation touches.
- **Parity-pinned (both legs):** with stdin redirected, `set_echo`
  returns `Err` (matching `enter_raw`'s and `is_raw`'s refusal — a
  non-terminal fd has no echo state to toggle).

## Deliberately no new surface (slice 2 scoping decision)

Two rusty_lines facets from the D9 survey get **no new trait method**,
because the existing surface already covers them:

- **Bracketed paste** is protocol bytes (`ESC[?2004h`/`l` to enable,
  content wrapped in `ESC[200~…ESC[201~`) read and written over the
  stream a consumer already controls via `read_chunk` — no OS call is
  involved, so encoding/decoding it stays the consumer's business, the
  same discipline this project applies to shell policy (rustils makes
  it *expressible*, does not own it).
- **Cooked↔raw suspend/resume** (the `$EDITOR`-handoff shape) is
  exactly `leave_raw()` then a later `enter_raw()` — their existing
  save/restore contract already produces the right outcome. Naming a
  separate `suspend`/`resume` pair would duplicate this surface, not
  extend it.

## Job-control terminal handoff (landed, rustils#43)

- `JobControlTerminal::give_terminal(pgid)` — `tcsetpgrp(STDIN_FILENO,
  pgid)` — hands the controlling terminal's foreground process group to
  `pgid`, or reclaims it for the caller's own group. A deliberately
  separate trait from `Terminal`: every backend (including Windows)
  implements `Terminal`, but there is no Windows equivalent for this one
  (D8: "no `fg`/`bg`/Ctrl-Z... no `tcsetpgrp` equivalent") — only
  `LinuxTerminal` implements `JobControlTerminal`.
- Sound only once `SIGTTOU` is ignored in the calling process (D1's
  precondition: a background process calling `tcsetpgrp` is stopped by
  `SIGTTOU` by default otherwise). The implementation encodes this
  itself — every `give_terminal` call first sets `SIGTTOU` to `SIG_IGN`
  (idempotent) — rather than documenting it as a caller obligation that
  could be forgotten.
- **Parity-pinned:** with stdin redirected (no controlling terminal),
  `give_terminal` returns `Err` — the same `ENOTTY` refusal
  `enter_raw`/`window_size` give on the same stream.
- **Live-verified only** (not parity-pinned, same discipline as
  `poll_readable`/`read_chunk`'s batching pin above): the real
  give/reclaim round-trip needs a live controlling terminal (a pty) —
  exercising it against a real job-control consumer is what proves the
  SIGTTOU-ignore precondition actually holds, not something CI's
  redirected-stream harness can observe.
- SIGTSTP/SIGCONT suspend-resume signal *delivery* stays out of this
  surface: `Child::kill_tree`/`kill_single`'s portable `Signal::Stop`/
  `Signal::Cont` (rustils#46, `docs/behavior/process.md`) already cover
  *sending* those signals to a job; this trait is only the
  terminal-ownership half.

## Console acquisition (landed 2026-08-05)

- `ConsoleAcquisition::alloc_console`/`attach_console`/`free_console` —
  `AllocConsole`/`AttachConsole`/`FreeConsole`, for a GUI-subsystem
  process that starts with no console at all. A deliberately separate
  trait from `Terminal`, mirroring `JobControlTerminal`'s own asymmetry
  in the other direction: only `WindowsTerminal` implements it (Unix has
  no GUI-subsystem/console-subsystem split to acquire a console into).
- `alloc_console`/`attach_console` always reopen `CONIN$`/`CONOUT$`
  (`CreateFileW`, immune to std-handle redirection) and install them via
  `SetStdHandle`, plus a best-effort VT-processing enable on the new
  stdout — every other `Terminal` method already implemented above
  observes the newly acquired console immediately, no separate fixup
  step. Needed because neither `AllocConsole` nor `AttachConsole`
  updates a std slot that was ever explicitly redirected — a documented
  Windows quirk (`docs/design-discussion-console.md` has the full
  story).
- `console_state()` reports `None`/`Inherited`/`Allocated`/`Attached` —
  instance state (like `enter_raw`'s saved-modes discipline), not a live
  OS probe (unlike `is_raw`'s deliberate live-probing): see
  `docs/design-discussion-console.md` for why that split is the right
  one here.
- `free_console` is idempotent — a no-op, not an error, when no console
  is currently attached, matching `leave_raw`'s own "without a prior
  enter it's a no-op" contract rather than surfacing whatever
  `FreeConsole` itself reports for "nothing to free".
- **Live-verified only** (CI-only, this crate's whole backend is
  cross-compile-checked from a Linux host — same discipline as the PTY
  surface's own Windows tests): `platform-windows/tests/
  console_acquisition.rs` exercises real `AllocConsole`/`AttachConsole`/
  `FreeConsole` calls against the test process's own console, serialized
  through a single mutex since acquisition is whole-process state, not
  per-instance.
- **Not landed**: `rusty_win32`'s `write_char_events`/`WriteConsoleInputW`
  synthetic-keystroke test harness (D9's own "test harness for free"
  bullet) — a distinct facet, waiting for a consumer that needs to drive
  a raw-mode reader through synthetic input in CI.

## Not in slice 1 or 2 (recorded, gated)

- PTY hosting (D13) is a distinct Process×Terminal surface (openpty vs
  ConPTY), not part of raw-mode/winsize.
- Byte→key decode (keymap policy — emacs/vi bindings — stays with the
  consumer, same as shell expansion/globbing stays with rush).

## Registered future divergence: resize notification

`rusty_term`'s backend (the D9 design oracle) proves this split is
real, not hypothetical: Unix has a genuine resize *event* (a SIGWINCH
delivery per change — see the Events D6 signal source, which this
surface would feed the identity into when it lands); Windows has
**no equivalent** — there is no console resize signal, so a consumer
must poll `window_size()` on a timer (`rusty_term` uses ~150ms) to
notice a change. `window_size()` itself stays a portable point-in-time
query (pinned above); *notification* of a change is not in slice 1 and
will carry this divergence when it is added — record it in
`docs/divergences.md` at that time rather than papering over the gap
with a fake Windows signal.
