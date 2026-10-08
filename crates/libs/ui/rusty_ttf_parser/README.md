# rusty-ttf-parser

A small, `#![no_std]`, allocation-free, dependency-free reader for the parts of
TrueType/OpenType fonts that text *shaping and colour rendering* need and an
outline rasteriser does not. It exists to replace `rusty_term`'s use of
[`ttf-parser`](https://crates.io/crates/ttf-parser), which is unmaintained
(RUSTSEC-2026-0192).

| Reads | API |
| --- | --- |
| Table directory, TrueType Collections | `Face::parse`, `Face::table_data` |
| `head`, `maxp` basics | `units_per_em`, `number_of_glyphs` |
| `cmap` formats 4, 6, 12 | `Face::glyph_index` |
| `GSUB`: features, lookups; types 1, 4, 5, 6; extension (7) | `Face::tables().gsub`, [`gsub`], [`opentype_layout`] |
| PNG bitmaps from `sbix` and `CBLC`/`CBDT` (formats 17, 18, 19) | `Face::glyph_raster_image` |

Outlines, advances and rasterisation are `rusty_font`'s job; the two crates are
complementary. This crate decodes lookups but does not *apply* them (the shaper
lives with the caller).

```rust
use rusty_ttf_parser::{Face, Tag};

let face = Face::parse(font_bytes, 0)?;
let gid = face.glyph_index('f');
if let Some(gsub) = face.tables().gsub {
    let liga = Tag::from_bytes(b"liga");
    for feature in gsub.features.into_iter().filter(|f| f.tag == liga) {
        for index in feature.lookup_indices.iter() {
            for subtable in gsub.lookups.get(index).into_iter().flat_map(|l| l.subtables()) {
                // match on `SubstitutionSubtable::{Single, Ligature, Context, ChainContext}`
            }
        }
    }
}
```

## Safety and robustness

- `#![forbid(unsafe_code)]`, no dependencies, no allocation.
- Every read is bounds-checked and returns `Option`/`Result`; malformed fonts
  degrade to "absent", never a panic.
- Loops over counts read from the font are clamped to what fits in the table
  (a 40-byte file cannot claim four billion strikes).
- `tests/robustness.rs` truncates and corrupts every byte of a real and several
  synthetic fonts and walks the whole API.

## How it was verified

Besides unit and fixture tests, the crate was checked differentially against
`ttf-parser` 0.25.1: every face of 201 fonts on the development machine
(Lato, Source Code Pro, FreeFont, Noto Color Emoji, WenQuanYi TTC collections,
icon fonts, ...) was dumped through both parsers and compared line by line:
about 789,000 `cmap` mappings, every GSUB feature, lookup and subtable
(single, ligature, context formats 2-3, chained context formats 1-3), and
19,670 colour-bitmap lookups across five sizes. All match. The comparison found
one real bug (a format 12 `cmap` lookup past the last group read the following
bytes), now a regression test.

Not covered by that corpus, so covered only by hand-built fonts in
`tests/synthetic.rs`: context lookup format 1 and `sbix`.

## Limits (deliberate)

GSUB lookup types 2, 3 and 8 are reported as `Unsupported`. No GPOS, GDEF,
variations, `cmap` format 13/14, `CBLC` index formats 4 and 5, or non-PNG
bitmap formats. Add them when a consumer needs them.

## License

MIT OR Apache-2.0, at your option (see the repository root).
