# Design discussion — Windowing + Registry/Config (Phase 9, nexus-only)

Not a decision record. `docs/convergence-roadmap.md`'s Phase 9 entry
has carried two provisional bullet points since the roadmap opened —
"Registry/Config first... a simple personality to formalize" and
"Windowing last... a thin pass-through with low near-term value" —
without ever getting the design pass most other phases got before
being scoped. This document is that pass: cloned `baileyrd/nexus`
read-only and read its actual Tauri-side source
(`shell/src-tauri/src/{persistence,windows}.rs`, ~750 lines total,
plus ADRs 0020 and 0033) rather than trusting the roadmap's own
two-line summary.

## Windowing: zero OS-syscall surface, not "thin," none

`shell/src-tauri/src/windows.rs` (BL-029, the multi-window/popout
surface) is the entirety of nexus's windowing code. Every operation —
`popout_window`, `close_popout_window`, `list_popout_windows`,
`get_popout_window_bounds`, `set_popout_window_bounds` — is a call
into `tauri::{WebviewWindowBuilder, WebviewWindow}`:
`.inner_size()`/`.position()`/`.decorations()`/`.resizable()` at
creation, `window.outer_position()`/`.inner_size()`/`.set_size()`/
`.set_position()`/`.close()` afterward. `shell/src-tauri/Cargo.toml`
has no `windows-sys`, `winapi`, `x11`, `wayland`, `cocoa`, or `objc`
dependency at all — confirmed by grep, not assumed. `lib.rs` layers
two more Tauri *plugins* on top
(`tauri_plugin_global_shortcut`, `tauri_plugin_window_state` — the
latter literally persisting window size/maximize/fullscreen across
launches, off the shelf) rather than hand-rolling anything.

**Verdict: there is no pass-through to be thin.** The roadmap's
existing "Tauri-mediated... thin pass-through with low near-term
value" undersells this — a thin pass-through still implies a real
call underneath worth wrapping. There is nothing underneath here that
`platform`'s own Layer 1 discipline (raw syscalls behind a curated
`ffi` surface, `docs/architecture.md`'s own layering) would ever
touch: no `CreateWindowW`, no X11/Wayland client code, no Cocoa
`NSWindow`. Tauri (via `tao`/`wry` beneath it) already *is* the
cross-platform layer for this domain, sitting structurally above
where `platform` operates, not beside it. A `platform::windowing`
trait mirroring `popout_window`/`set_popout_window_bounds` would
re-wrap Tauri's own API one level up for zero OS-abstraction benefit —
there is no lower-level OS fact left to extract.

## Registry/Config: one real gap, and nexus doesn't have it

`shell/src-tauri/src/persistence.rs` (`shell-state.json`, ADR 0033):
a single JSON file, loaded once, atomically rewritten (temp file +
`fs::rename`), guarded by a module-scoped advisory `Mutex` so
concurrent multi-window writes don't lose a mutation to a
load-modify-save race (`SHELL_STATE_WRITE_LOCK`/`with_lock_update`,
live-tested with 16 concurrent threads in `persistence.rs`'s own test
suite). Two things worth separating:

- **The atomic-write half is already solved, here, today.**
  `Dir::write_atomic` (D11, landed 2026-07-19) is exactly this pattern
  — temp name in the same directory, `sync_all`, `rename` over the
  target — already cited in `extraction-map.md`'s own cross-cutting
  notes as appearing "twice" across the ecosystem (nexus
  `storage/atomic.rs`, rusty_naner's staged install). `persistence.rs`
  is functionally a third occurrence of the identical pattern nexus's
  own `storage/atomic.rs` already represents in that note — nothing
  new to extract.
- **The concurrent-write mutex is nexus's own policy, not an OS
  fact.** An in-process `Mutex` serializing a specific Tauri command's
  read-modify-write sequence is application logic about *this
  consumer's* concurrency model (multiple windows sharing one kernel
  process), not a platform capability — the same "expressible, not
  owned" boundary this codebase already draws for shell policy and
  bracketed paste.

**The one piece that would be a genuine `platform::fs` gap — and
nexus doesn't actually have it.** `resolve_path` in `persistence.rs`
calls `app.path().app_config_dir()` — **Tauri's own API**, not a
hand-rolled `dirs`-crate-style lookup the roadmap's "file-backed JSON
+ `dirs` paths" phrasing implies. `platform::fs` has no equivalent at
all (`Dir::open_ambient` always takes a caller-supplied absolute
path — nothing answers "what's the standard per-user config directory
on this OS," the `$XDG_CONFIG_HOME`/`~/.config` vs `%APPDATA%`
question). That is a real, missing, generalizable OS fact. But nexus
itself has no gap here to force it: Tauri already answers the question
nexus asks, with no `platform`-shaped hole in nexus's own code at all.

## The `CredentialVault` pattern, again

This is the same verdict `docs/design-discussion-sandbox.md` reached
for nexus's `CredentialVault`: a complete, working, framework-mediated
implementation with no gap, no TODO referencing rustils, and no
expressed desire to migrate. Registry/Config isn't "a simple
personality to formalize" the way the roadmap currently frames it —
it's donor material with nothing to formalize *for this consumer*,
because the framework nexus already depends on solved the OS-fact
question before `platform` ever could.

## What this reshapes about Phase 9

Windowing has no extractable surface at all — stronger than
"deprioritized," there may be no PAL work here ever, for this
consumer or a hypothetical similar one (any Tauri-based nexus-shaped
app hits the identical "Tauri already solved it" wall). Registry/Config
has exactly one real, narrow, generalizable gap (known-directory
resolution), but the named consumer that was supposed to force it
doesn't actually need it forced. Phase 9 as currently scoped
("Windowing + Registry/Config, nexus-only, lands here + nexus") rests
on a premise this document's own reading of nexus's real code doesn't
support.

The closest precedent isn't Sandbox's "two different problems"
finding — it's simpler and flatter: **`rusty_lsp` is already recorded
in this codebase as "the counter-example that validates the gate:
zero platform crates; it converges by doing essentially nothing"**
(`extraction-map.md`'s cross-cutting notes). Phase 9, on this
document's evidence, is nexus's own version of that same
counter-example for both of its named facets, not a phase with real
work waiting to be sequenced.

## Open questions for the owner, not decided here

1. **Close Phase 9 with no PAL surface for either facet** — the
   `rusty_lsp` precedent applied here — or keep it open on the chance
   a future nexus feature (or a different, not-yet-named consumer)
   eventually needs either?
2. **Build known-directory resolution speculatively anyway**, the
   same posture PTY hosting and console acquisition were built under
   (owner's explicit call, no confirmed consumer) — worth it as a
   small, self-contained, genuinely-missing OS-fact primitive even
   though nexus itself doesn't force it, or does building capabilities
   with zero forcing consumer left twice in one week (this and the
   msys-parity family, both parked) suggest the gate is working as
   intended and this should wait too?
3. **Coverage honesty check**: this document read `persistence.rs`,
   `windows.rs`, and ADRs 0020/0033 — not nexus's entire tree. Is
   there a different nexus surface (or a different donor entirely)
   with real OS-level windowing or config semantics this selection
   missed? Asked directly rather than assumed away.

## What this document does not decide

Whether to close Phase 9, build known-directory resolution anyway, or
look further before concluding either way. Same posture as every
document in this family: input to an owner's call, not the call
itself.
