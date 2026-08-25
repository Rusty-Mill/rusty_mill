# rusty_font

A `#![no_std]` + `alloc` sovereign TrueType/OpenType font table parser and
glyph rasterizer for the **Rusty Mill** ecosystem.

## What's real

Previously a total stub: `Font::parse` never read the file (hardcoded
`units_per_em: 2048, num_glyphs: 256`), `glyph_index`/`glyph_outline`
returned a fixed placeholder box, and the rasterizer filled a hardcoded
rectangle regardless of input. All of that is now real:

- **`ttf.rs`** — a real `sfnt` table directory parser: `head`/`maxp`
  metadata, `cmap` format-4 (the segment-based Unicode BMP mapping every
  Latin-script TrueType font ships) with the spec's exact pointer
  arithmetic, `cmap` format-12 (segmented coverage over the full 21-bit
  Unicode range, used by fonts with supplementary-plane glyphs — verified
  against a real Nerd Fonts icon set), `loca`/`glyf` simple-glyph outline
  extraction (contours, on/off-curve quadratic points, run-length-encoded
  flags and coordinate deltas), and composite-glyph assembly (component
  records resolved recursively and transformed — scale, x/y scale, or a
  full 2x2 matrix, plus an offset — into the composite's coordinate
  space).
- **`rasterizer.rs`** — a real scanline rasterizer: flattens TrueType's
  on/off-curve quadratic Bézier contours into line segments, then fills
  with the **non-zero winding rule** (the rule TrueType itself specifies —
  required for a glyph like `O` to render with its counter/hole actually
  unfilled, not solid).

**Verified against real system fonts** where the machine running the tests
has them: `cargo test` parses `arial.ttf`/`cour.ttf` from
`C:\Windows\Fonts`, resolves real character-to-glyph mappings, extracts
real outlines (checking e.g. that `A` has ≥2 contours — the outer shape
plus its triangular counter), and rasterizes them — including a direct
test that `O`'s rendered center pixel is unfilled (the hole) while its
ring is filled, proving the winding rule is genuinely correct, not just
"produces some pixels." Behavior that a real system font can't exercise
on demand (a font shipping a `cmap` format-12 subtable, a specific
composite-glyph transform) is instead tested against a minimal
synthetic `sfnt` built directly in the test, byte for byte, rather than
skipped. `examples/ascii_render.rs` dumps real rasterized letters as
ASCII art for a visual sanity check:

```
cargo run --example ascii_render
```

## Known, deliberate gaps

- **Composite-glyph point matching** (`ARGS_ARE_XY_VALUES` unset — a
  component positioned by matching a point on it to a point on a
  previously-placed component, rather than by an explicit x/y offset) is
  rare in real-world fonts and isn't implemented; that one component is
  skipped rather than fabricating a position for it, while the rest of
  the composite still assembles.
- **CFF-outline OpenType** (`OTTO` version tag) isn't supported — only
  TrueType `glyf`-outline fonts.
- **No hinting** — outlines rasterize at their natural shape; no
  instruction execution or grid-fitting.

## Testing

```
cargo test
cargo run --example ascii_render
```
