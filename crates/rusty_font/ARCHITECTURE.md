# Architecture

## Overview

`rusty_font` is a `#![no_std]` + `alloc` library, not a service: it parses
`sfnt`-container TrueType/OpenType font bytes into glyph outlines
(`ttf.rs`) and rasterizes those outlines into filled pixel coverage
(`rasterizer.rs`). There is no I/O of its own — callers supply the raw
font bytes and consume the rasterized output; anything resembling file or
network access (loading `arial.ttf`, etc.) lives in the caller or in
tests/examples, not in the crate.

## Boundaries

The crate is a pure data transform, so the ports-and-adapters split doesn't
apply in its usual I/O sense — there's no filesystem/network adapter to
swap. The boundary that does matter is internal, between parsing and
rendering:

| Stage | Module | Notes |
| ---- | ---------- | ----- |
| Table parsing (`sfnt` container) | `ttf.rs` | Reads `sfnt` table directory, `head`/`maxp`, `cmap` (format 4 and format 12) — bytes in, `Font` out. Dispatches glyph outline extraction to one of two sources depending on the font's `sfnt` version tag. No rendering knowledge. |
| Outline extraction (TrueType) | `ttf.rs` | `loca`/`glyf` — simple and composite glyphs, producing exact on/off-curve quadratic points. |
| Outline extraction (CFF) | `cff.rs` | `CFF ` table INDEX/DICT parsing and a Type 2 charstring interpreter, producing an *approximated* outline — cubic Bézier curves flattened to on-curve line segments, since the shared outline model is quadratic-point-shaped. Never touches `ttf.rs`'s TrueType path or `rasterizer.rs`. |
| Outline model | `glyph.rs` | `Point`/`GlyphOutline` — the shared representation both outline sources produce and rasterizing consumes. Holds no logic. |
| Rasterization | `rasterizer.rs` | Flattens quadratic Bézier contours to line segments and fills via non-zero winding. Takes a `GlyphOutline`, has no knowledge of font file formats or which outline source produced it. |

Every stage only communicates through `glyph.rs`'s types, which is what
keeps format-parsing changes (adding `cmap` format 12, assembling
composite glyphs, adding the whole CFF path) from touching rasterization
and vice versa — composite assembly, in particular, is `ttf.rs`
recursively resolving and transforming other `ttf.rs` outlines, and CFF
parsing is entirely self-contained in `cff.rs`, both without touching
`rasterizer.rs`.

## Structure

Single-crate library (not a modular monolith split into components) —
appropriate at this size; a split would be premature. See
[docs/adr/](./docs/adr/) if a future forcing function (e.g. a separate
SIMD backend crate) changes that.

## Data flow

1. Caller passes raw font bytes to `Font::parse` (`ttf.rs`).
2. `Font::parse` walks the `sfnt` table directory, reads `head`/`maxp` for
   metadata, and — based on the `sfnt` version tag — either records the
   `loca`/`glyf` table ranges (TrueType) or hands the `CFF ` table to
   `cff::parse_cff_table` (OpenType/CFF).
3. `Font::glyph_index` maps a Unicode codepoint to a glyph ID via `cmap`
   (format 4 or 12) — independent of which outline source the font uses.
4. `Font::glyph_outline` resolves the glyph ID into a `GlyphOutline`:
   through `loca`/`glyf` for TrueType, or through `cff::glyph_outline`'s
   Type 2 charstring interpreter for CFF.
5. `Rasterizer` consumes the `GlyphOutline`, flattens its curves, and fills
   the result into a pixel buffer using the non-zero winding rule — the
   same code path regardless of which outline source produced it.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals

- Composite-glyph point matching (a rare positioning mode), CID-keyed CFF,
  `seac`-style accent composition, and hinting are known gaps — see the
  README's "Known, deliberate gaps" section, not silent omissions.
- File/network loading of font data is explicitly out of scope — the crate
  takes bytes, however the caller obtained them.
