# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

One entry per merged PR against `main`, most recent first, each linking to its PR.

---

## PR #6 — Flesh out the Windows event pump: resize, full keyboard, text input, mouse wheel/right/middle, redraw

**2026-08-25** · [#6](https://github.com/baileyrd/rusty_gui/pull/6)

- **Added:** `Window::poll_events` (Windows backend) now handles `WM_SIZE`
  (`Event::Resized`, and `width()`/`height()` now reflect the live size),
  `WM_KEYUP` (`Event::KeyReleased`) alongside the existing `WM_KEYDOWN`, a full
  virtual-key table (letters, digits, arrows, Backspace/Tab/Return/Space, not
  just Escape), `WM_CHAR` (`Event::ReceivedCharacter`, including surrogate-pair
  decoding), `WM_RBUTTONDOWN/UP` and `WM_MBUTTONDOWN/UP` (right/middle mouse
  buttons), `WM_MOUSEWHEEL` (`Event::MouseWheel`), and `WM_PAINT`
  (`Event::RedrawRequested`). `Window::request_redraw` now actually calls
  `InvalidateRect` instead of being a no-op.
- **Added:** `Event::MouseWheel`, `Event::ReceivedCharacter`,
  `Event::ModifiersChanged`, `KeyCode::{Shift,Control,Alt}`, and a new
  `ModifiersState` type tracking which modifier keys are currently held.
  Purely additive — no existing public signature changed.
- **Known limitation, stated plainly:** real IME composition (preedit text,
  candidate windows) is still not implemented — `WM_CHAR` only covers
  already-composed UTF-16 text. Full IME support needs its own follow-up
  (`WM_IME_*` message handling and new `Event` variants for composition
  state), tracked as remaining scope on issue [#2](https://github.com/baileyrd/rusty_gui/issues/2).
- Also expands CI to a `[ubuntu-latest, windows-latest]` matrix — the
  previous ubuntu-only job never even compiled this crate's `#[cfg(windows)]`
  code, so it would have caught nothing here.
- 5 new unit tests for the virtual-key-to-`KeyCode` mapping table (run on the
  `windows-latest` CI leg — this logic doesn't compile on other targets).

## PR #5 — Add repo-config governance files; fix pre-existing clippy warnings

**2026-08-25** · [#5](https://github.com/baileyrd/rusty_gui/pull/5)

- **Added:** standard governance file set (README, CONTRIBUTING, CODE_OF_CONDUCT,
  SECURITY, CHANGELOG, RELEASE_NOTES, ARCHITECTURE, an ADR seed, issue/PR
  templates, `.gitattributes`, and a `ci-rust.yml` GitHub Actions workflow).
- **Fixed:** two pre-existing `cargo clippy -D warnings` failures
  (`unused_mut` in `Window::poll_events`, `dead_code` on the placeholder
  `x11_window` field) that would otherwise have kept the new CI workflow red
  on every PR, including this one.
- No behavior change to the crate's public API.
