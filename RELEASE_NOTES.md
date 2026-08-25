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

Tracks notable changes to `rusty_font`, one entry per merged PR against
`main`, reverse chronological (no version tags yet, so PRs are the unit of
change).

---

## PR — Decompose composite glyphs instead of returning a bounding box
**2026-08-25**

- **Added:** composite-glyph assembly (`ttf.rs`) — component records
  (flags, referenced glyph index, x/y offset, optional scale/2x2
  transform) are parsed per the `glyf` spec, each referenced glyph
  resolved recursively via the existing simple-glyph path, transformed,
  and concatenated into the composite's outline (`contour_ends` offset by
  the running point count per component). `Font::glyph_outline` now
  returns the real assembled shape for characters like `é`/`ñ`/`ü` instead
  of an empty bounding box (per [#4](https://github.com/baileyrd/rusty_font/issues/4)).
- **Changed:** a recursion depth cap (8 levels) guards against a
  malformed/cyclic composite font; beyond it (or on genuinely malformed
  component data), the glyph gracefully degrades to the previous
  bounding-box-only outline rather than failing the lookup.
- Known remaining gap: components using point-matching
  (`ARGS_ARE_XY_VALUES` unset, rare in real-world fonts) are skipped
  rather than fabricating a position — documented in the README.
- 3 new unit tests build a minimal synthetic `sfnt` font directly (simple
  glyphs + a referencing composite), since assembling a controlled
  multi-component shape isn't something a real system font lets a test
  dictate: offset-only assembly across two components, a scale transform,
  and the point-matching-component-is-skipped fallback. 20 total, 0
  failed.
- Closes [#4](https://github.com/baileyrd/rusty_font/issues/4).
- Link will be added once the PR is open.

## PR #7 — Support cmap format 12 (supplementary-plane Unicode)
**2026-08-25** · [#7](https://github.com/baileyrd/rusty_font/pull/7)

- **Added:** `cmap` format-12 subtable parsing (`ttf.rs`) — segmented
  coverage over the full 21-bit Unicode range, used by fonts that carry
  supplementary-plane glyphs (e.g. Nerd Fonts' Material Design icon
  ranges above U+FFFF, per [#3](https://github.com/baileyrd/rusty_font/issues/3)).
  `Font::glyph_index` now resolves those codepoints instead of returning
  `None`. When a font ships both a format-4 and a format-12 subtable,
  format 12 is preferred (it's a strict superset of format 4's BMP-only
  coverage).
- 2 new unit tests (constructing synthetic `cmap` tables directly, since
  the existing system-font-backed tests only exercise fonts already on
  the machine): group lookup at the group's start/middle/end and just
  past it, and format-12-over-format-4 preference when both are present.
  17 total, 0 failed.
- Closes [#3](https://github.com/baileyrd/rusty_font/issues/3).

## PR #6 — Add standard governance files
**2026-08-25** · [#6](https://github.com/baileyrd/rusty_font/pull/6)

- **Added:** repo-config's standard governance set — PR/issue templates,
  CONTRIBUTING/CODE_OF_CONDUCT/SECURITY/CHANGELOG/RELEASE_NOTES/ARCHITECTURE,
  an ADR seed, `.gitattributes` (forces LF), and a Rust CI workflow
  (`fmt` + `clippy -D warnings` + `test`). README was left as-is (already
  present). ARCHITECTURE's boundary table and data-flow were hand-written
  for this crate's actual parse → outline → rasterize pipeline rather than
  left as scaffold.
- **Changed:** ran `cargo fmt --all` across the existing source
  (`src/ttf.rs`, `src/rasterizer.rs`, `examples/ascii_render.rs`) —
  formatting only, no behavior change — so the new CI's `fmt --check` gate
  doesn't start out red against unformatted pre-existing code.
- 15 unit tests unaffected: 15 passed, 0 failed.
