# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Standard repo-config governance file set: README, ARCHITECTURE,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, RELEASE_NOTES, ADR seed,
  issue/PR templates, `.gitattributes`, `ci-rust.yml`.
### Changed
### Fixed
- `cargo fmt`/`cargo clippy -D warnings` baseline failures surfaced by the new
  CI workflow (unformatted code; `Framebuffer::present`'s `window` param
  unused on non-Windows targets).
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
