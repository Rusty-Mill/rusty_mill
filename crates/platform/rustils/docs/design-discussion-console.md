# Design discussion — console acquisition (D9's rusty_naner facet)

Not a decision record — the same kind of design pass
`docs/design-discussion-pty.md` held before PTY hosting landed, for the
same underlying reason: `docs/extraction-map.md`'s D9 entry names the
donor (`rusty_naner`'s `naner-core/console.rs`) and the shape in one
sentence ("AttachConsole/AllocConsole/CONOUT$ reopen/enable-VT with a
`ConsoleState` enum") but this session has no checkout of `rusty_naner`
itself to verify variant names or exact call sequencing against. What
*is* in scope: `rusty_win32`, the other D9 donor, whose `src/console.rs`
exposes the raw `AllocConsole`/`FreeConsole`/`AttachConsole` primitives
(previously unused by this repo) alongside the raw-mode/winsize
primitives `platform-windows::sys::console` already ported. This slice
builds the portable trait from *that* donor's real primitives plus
ordinary Win32 domain knowledge for the parts D9's one-sentence
description didn't spell out (exact `ConsoleState` variants, whether
`CONOUT$` reopen is bundled into acquisition or left to the caller) —
flagged here rather than presented as a verified port, the same honesty
`docs/design-discussion-pty.md` applied to its own D13 gap.

## Outcome

**Landed 2026-08-05** — the owner's explicit call, same posture as PTY
hosting (D13) and `Sandbox`'s confinement half: built without a confirmed
live consumer (no shell or terminal-emulator consumer has asked for this
yet), accepting the same speculative-build risk those slices were held
for explicitly.

## What D9 already established, and what it didn't

D9's own text bundles four things into one bullet: `AttachConsole`,
`AllocConsole`, a `CONOUT$` reopen, and "enable-VT" — plus a
`ConsoleState` enum whose variants are never named in this repo. Two of
those four (`AllocConsole`/`AttachConsole`) are real, tested primitives
in the `rusty_win32` donor this session *does* have (`src/console.rs`'s
`alloc`/`attach`/`free`, `ATTACH_PARENT_PROCESS`); the `CONOUT$`
reopen appears in that same donor file, but only inside its own
`#[cfg(test)]` module (`open_console`, used by `ensure_console_stdin` to
get a real console handle for its raw-mode tests) — never promoted to
production API there. `rusty_win32`'s own comment on `open_console`
explains exactly why a reopen is needed at all: `GetStdHandle` keeps
returning a stale/redirected handle even after `AllocConsole` gives the
process a real console, a documented Windows quirk found the hard way on
`windows-latest` CI.

## The shape question this document resolves

**Does console acquisition end at `AllocConsole`/`AttachConsole`
succeeding, or does it also fix up std handles so the rest of
`platform::term::Terminal` works immediately afterward?**

D9's own bullet already answers this by bundling "CONOUT$ reopen/
enable-VT" into the same sentence as the acquisition calls themselves,
and the donor's own quirk (above) makes the alternative actively
misleading: a caller that acquired a console but still can't `is_tty`/
`enter_raw`/`read_chunk` against it via the existing `Terminal` surface
would consider acquisition to have silently failed. **Decision:
`alloc_console`/`attach_console` always reopen `CONIN$`/`CONOUT$` and
install them via `SetStdHandle` before returning `Ok`, plus a
best-effort VT-processing enable on the new stdout** — matching
`enter_raw`'s own already-established "VT is best-effort, everything
else is not" posture (`platform-windows/src/sys/console.rs::enter_raw`).
Windows has no third `CONERR$` device name; stdout and stderr are two
independent handles opened against the same `CONOUT$` screen buffer.

## `ConsoleState`: what the four variants had to cover

`rusty_naner`'s real enum isn't available to port, so this slice derives
one from what a caller actually needs to distinguish, cross-checked
against every real console-related failure mode the two donors'
primitives can produce:

- `None` — no console at all (the GUI-subsystem default, or the state
  right after `free_console`). `alloc_console`/`attach_console` are the
  only two operations valid from here.
- `Inherited` — a console was already attached at process creation (the
  ordinary console-subsystem case) — every other `Terminal` capability
  already works against it with zero acquisition calls, which is exactly
  why this state needs to exist separately from `Allocated`/`Attached`:
  a caller checking `console_state()` before deciding whether to acquire
  one needs to tell "already have a usable console" apart from "have
  none yet".
- `Allocated` / `Attached` — which of the two acquisition mechanisms
  produced the process's current console, kept distinct because they
  have different failure/no-op implications for a caller (e.g. "was this
  console created just for us, or is it shared with a parent" affects
  whether closing it on exit is this process's call to make — a policy
  decision deliberately left to the consumer, not decided here).

Not modelled as a fifth "redirected" variant some framings of the
"attach-vs-alloc-vs-redirected" personality (see D9's own phrasing) might
expect: a *redirected* std handle (pipe/file) is already
`Terminal::is_tty` returning `false` on that stream — an orthogonal,
already-answered question this trait doesn't need to re-encode as a
`ConsoleState` variant of its own.

## Instance state, not a live probe — and why that's the right call here

`console_state()` returns a field on `WindowsTerminal`, updated only by
this same handle's own `alloc_console`/`attach_console`/`free_console`
calls — not re-queried from the OS on every call. This mirrors
`enter_raw`/`leave_raw`'s own save/restore instance-state discipline
(not `Terminal::is_raw`'s *deliberately* live-probing one): raw mode's
`is_raw` needs to be live because *external* processes commonly mutate
console/termios modes behind this handle's back (`stty`, a suspended-
then-foregrounded shell — `docs/behavior/term.md`'s own rationale).
Console *attachment*, by contrast, is not something another process
routinely changes out from under this one mid-session — the realistic
mutator is this same handle's own prior call, which instance state
already tracks correctly. A future consumer that needs live drift
detection here (unlikely, but not ruled out) can add it without breaking
this contract, the same way `is_raw` was added as its own explicit
method rather than folded into a cached `enter_raw` flag.

## CI-caught bug: `GetConsoleWindow` is the wrong "has a console" probe

The first version of `initial_state()`'s seed probe used `GetConsoleWindow`
(non-null `HWND` ⇒ a console is attached). `windows-latest` CI (PR #92)
caught why that's wrong: the runner's `pwsh` invocation for `cargo test`
is attached to a real console (`AllocConsole` correctly refused with
`ERROR_ACCESS_DENIED` when a test tried to allocate a second one) that
nonetheless has **no `HWND`** — the same "ConPTY-hosted consoles have no
window" fact this crate's own `platform::pty` backend already
established (`docs/design-discussion-pty.md`). `GetConsoleWindow`
returning null therefore does not mean "no console"; it only means "no
*windowed* console," and CI's own runner shape falsified the assumption
immediately. Fixed by probing attachment the same way
[`reopen_std_handles`] already opens `CONIN$`/`CONOUT$` — open `CONIN$`
and check whether it succeeds, which is true for both windowed and
window-less (ConPTY-hosted) consoles alike. `sys::console::has_console`
carries the full story in its own doc comment. The lesson generalizes:
this crate's Windows backend already treats "does this look like a
console app" as a `GetConsoleMode`-succeeding question rather than a
window-existence one everywhere else (`sys::console::is_tty`'s own
doc comment says so explicitly) — the acquisition-state probe should
have followed the same discipline from the start rather than reaching
for `GetConsoleWindow` as a shortcut.

## What stays out of scope

- **Unix**: no analog at all — every Unix process either inherits a
  controlling terminal at exec time or has none; there is no
  GUI-subsystem/console-subsystem split, and therefore no "go acquire a
  console" step to model. `ConsoleAcquisition` has exactly one real
  implementor (`WindowsTerminal`), the same asymmetry
  `JobControlTerminal` already establishes in the opposite direction
  (Unix-only, no Windows implementor).
- **`rusty_win32`'s test-harness primitives** (`write_char_events`/
  `write_key_events`/`WriteConsoleInputW` — synthesizing keystrokes into
  a console input buffer, D9's own "test harness for free" bullet): a
  distinct facet from acquisition, waiting for a consumer that actually
  needs to drive a raw-mode reader through synthetic input in CI (e.g. a
  future `rusty_lines` convergence) rather than landing speculatively
  alongside this slice.
- **`platform-mock`**: no `MockConsoleAcquisition` counterpart, matching
  `JobControlTerminal`'s own precedent — an opt-in, effectively
  single-backend trait doesn't get a scripted double just because one is
  offered for the portable `Terminal` surface itself.
- **Window/title/codepage/fill/cursor primitives** (`rusty_win32`'s
  `title`/`set_title`/`set_input_codepage`/`fill_char`/
  `set_cursor_position`/etc.): real, tested donor material, but a
  distinct facet from *acquisition* — parked until a consumer (a
  full-screen console UI, most plausibly) forces it, the same D9
  discipline every other still-deferred facet in this cluster follows.
