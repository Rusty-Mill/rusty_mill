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

## Repo governance setup, and Pipeline coverage blit + clipping (closes #2)
**2026-08-25** · (not yet merged — link added once this lands)

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
