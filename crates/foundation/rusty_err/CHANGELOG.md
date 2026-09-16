# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
- `BoxError` is now `Send + Sync` unconditionally (previously depended on
  what was boxed inside), so error types embedding one can be used across
  `#[async_trait]` boundaries. ([#4](https://github.com/baileyrd/rusty_err/issues/4))
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
