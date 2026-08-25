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
| Table parsing | `ttf.rs` | Reads `sfnt` table directory, `head`/`maxp`, `cmap` format 4, `loca`/`glyf` — bytes in, `Font`/`GlyphOutline` out. No rendering knowledge. |
| Outline model | `glyph.rs` | `Point`/`GlyphOutline` — the shared representation between parsing and rasterizing. Holds no logic. |
| Rasterization | `rasterizer.rs` | Flattens quadratic Bézier contours to line segments and fills via non-zero winding. Takes a `GlyphOutline`, has no knowledge of font file formats. |

`ttf.rs` and `rasterizer.rs` only communicate through `glyph.rs`'s types,
which is what keeps format-parsing changes (e.g. adding `cmap` format 12)
from touching rasterization and vice versa.

## Structure

Single-crate library (not a modular monolith split into components) —
appropriate at this size; a split would be premature. See
[docs/adr/](./docs/adr/) if a future forcing function (e.g. a separate
SIMD backend crate) changes that.

## Data flow

1. Caller passes raw font bytes to `Font::parse` (`ttf.rs`).
2. `Font::parse` walks the `sfnt` table directory and reads `head`/`maxp`
   for metadata.
3. `Font::glyph_index` maps a Unicode codepoint to a glyph ID via `cmap`
   format 4.
4. `Font::glyph_outline` resolves the glyph ID through `loca`/`glyf` into a
   `GlyphOutline` (contours of on/off-curve points).
5. `Rasterizer` consumes the `GlyphOutline`, flattens its curves, and fills
   the result into a pixel buffer using the non-zero winding rule.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals

- Composite glyph assembly, `cmap` format 12 (non-BMP Unicode), CFF/`OTTO`
  outlines, and hinting are known gaps — see the README's "Known,
  deliberate gaps" section, not silent omissions.
- File/network loading of font data is explicitly out of scope — the crate
  takes bytes, however the caller obtained them.
