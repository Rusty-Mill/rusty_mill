# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Standard repo-config governance file set: README, ARCHITECTURE,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, RELEASE_NOTES, ADR seed,
  issue/PR templates, `.gitattributes`, `ci-rust.yml`.
- `Pipeline::blit_coverage`: alpha-blended (source-over) compositing of an
  8-bit coverage mask onto a `Framebuffer`, for glyph/coverage-bitmap
  rendering (closes #2).
- `ClipRect` + `Pipeline::set_clip`/`clip`: constrains `draw_rect` and
  `blit_coverage` to a sub-rectangle of the framebuffer (closes #2).
- `Color::from_u32`, `Framebuffer::get_pixel`: pixel read-back, backing the
  new blend path.
### Changed
### Fixed
- `cargo fmt`/`cargo clippy -D warnings` baseline failures surfaced by the new
  CI workflow (unformatted code; `Framebuffer::present`'s `window` param
  unused on non-Windows targets).
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
