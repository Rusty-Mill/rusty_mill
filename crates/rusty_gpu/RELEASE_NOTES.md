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

One entry per merged PR against `main`, reverse chronological. No version tags
published yet (pre-1.0).

---

## Linux X11 presentation path (partial fix for #3)
**2026-08-25** · (not yet merged — link added once this lands)

- **Added:** `Framebuffer::present` now does a real blit on Linux — hand-rolled
  raw Xlib FFI (`XOpenDisplay`/`XCreateImage`/`XPutImage`, no `x11`/`xcb` crate
  dependency), matching how `rusty_win32`'s Windows path and `rusty_gui`'s own
  Linux window backend call their platform APIs raw. Opens its own `Display`
  connection per call (mirrors `blit_pixel_buffer`'s per-call `GetDC`/`ReleaseDC`
  pattern) rather than reusing `rusty_gui::Window`'s private one, which isn't
  exposed — a second connection to the same X server drawing onto a window it
  doesn't own is a standard, supported X11 pattern.
- **Changed:** bumped the `rusty_gui` git pin to
  [10954f5](https://github.com/baileyrd/rusty_gui/commit/10954f56b7f700538827421b2f60c7f3e1958684)
  (merges [rusty_gui#7](https://github.com/baileyrd/rusty_gui/pull/7), the real
  Linux/X11 window backend) — the previous pin's `Window::raw_handle()` returned
  a null pointer on Linux, which is what made this genuinely blocked before.
- **Fixed:** CI's first run on this change failed — `rust-lld: error: unable
  to find library -lX11`. `ubuntu-latest` doesn't ship `libX11` by default;
  linking against it directly (`#[link(name = "X11")]`) is a real link-time
  requirement even though the runner has no display to blit to. `ci-rust.yml`
  now installs `libx11-dev` before building.
- **Verified against a real (virtual) X server**, not just build+clippy: ran a
  throwaway example under `Xvfb`, blitted a solid background plus an
  off-center rect via `Pipeline::draw_rect`, then read pixels back with a
  second `XGetImage` connection — background, inside the rect, just outside
  the rect on the same row (catches a `bytes_per_line`/stride bug), and the
  opposite corner all matched exactly. Not committed as an automated test:
  it needs a live X server, which this repo's CI (`ubuntu-latest`, headless)
  doesn't have — same reason `present()` has never had an automated test on
  the Windows side either.
- **Known limitation, stated plainly:** assumes the display's default visual
  is a standard 24/32-bit-depth TrueColor visual in the host's native byte
  order (true for essentially every modern Linux desktop) — an exotic visual
  will blit with wrong/garbled colors rather than being detected and
  rejected. Opens/closes a fresh `Display` connection every call rather than
  caching one, a real (if currently unmeasured) per-frame cost; documented
  as a candidate optimization, not fixed here to keep this change simple and
  free of global mutable state.
- **Issue #3 stays open, partially:** macOS remains genuinely blocked — no
  Cocoa/AppKit backend exists in `rusty_gui` yet, and `rusty_gui`'s own
  [#3](https://github.com/baileyrd/rusty_gui/issues/3) tracks it as
  out-of-scope for the PR that added the Linux backend.

---

## Repo governance setup, and Pipeline coverage blit + clipping (closes #2)
**2026-08-25** · [#4](https://github.com/baileyrd/rusty_gpu/pull/4)

- **Added:** applied the standard repo-config governance file set (README,
  ARCHITECTURE, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG,
  RELEASE_NOTES, ADR seed, issue/PR templates, `.gitattributes`,
  `ci-rust.yml`) — prerequisite for issue-loop to start working this repo's
  open issue backlog.
- Filled README/ARCHITECTURE with real content (boundary table, data flow)
  rather than leaving the scaffold placeholders.
- **Fixed:** `cargo fmt`/`cargo clippy -D warnings` baseline failures (unformatted
  code, `Framebuffer::present`'s `window` param unused off-Windows) — pre-existing,
  surfaced by the new `ci-rust.yml` gate; fixed so the "on green CI, merge" rule
  has a working baseline to gate on.
- **Added:** `Pipeline::blit_coverage` — alpha-blended (source-over) compositing
  of an 8-bit coverage mask (e.g. a `rusty_font::Rasterizer` glyph bitmap) onto
  a `Framebuffer`, instead of `set_pixel`'s unconditional overwrite. Backed by
  new `Color::from_u32`/`Framebuffer::get_pixel` to read back existing pixel
  content to blend against.
- **Added:** `ClipRect` + `Pipeline::set_clip`/`clip` — constrains `draw_rect`
  and `blit_coverage` to a sub-rectangle of the framebuffer (e.g. for split
  panes/scroll regions in a terminal-style compositor).
- 8 new unit tests (fill, clip constraint on both draw paths, full/zero/partial
  coverage blending, mismatched-buffer-length panic); all passing.
- **Known limitation, stated plainly:** no SIMD acceleration yet for
  `blit_coverage`'s per-pixel blend loop, despite this crate depending on
  `rusty_simd` — `rusty_simd` currently only exposes generic elementwise ops
  (`vec_add`/`vec_mul`), nothing coverage-blend-shaped to build on without
  first adding it there. Left as scalar Rust; issue #3 (Windows-only
  presentation) remains open and unrelated to this change.
