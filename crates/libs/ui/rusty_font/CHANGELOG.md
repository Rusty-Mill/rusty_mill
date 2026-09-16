# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Standard governance file set (PR/issue templates, CONTRIBUTING,
  CODE_OF_CONDUCT, SECURITY, ARCHITECTURE, ADR seed, `.gitattributes`,
  Rust CI workflow) via repo-config.
- `cmap` format-12 subtable support (supplementary-plane Unicode
  codepoints, e.g. Nerd Fonts icon ranges above U+FFFF); preferred over
  format 4 when a font ships both.
- Composite-glyph assembly: `Font::glyph_outline` decomposes and
  transforms component glyphs (scale/2x2 matrix + offset) instead of
  returning a bounding box with no points.
- CFF-flavor OpenType (`OTTO`) support: a new `cff.rs` module parses the
  `CFF ` table and interprets Type 2 charstrings, flattened to line
  segments, so `Font::parse`/`glyph_outline` work on CFF-outline fonts
  instead of rejecting them as an unsupported version.
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
