# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `rusty_serde::json::to_value`/`from_value`: convert a `Serialize`/`Deserialize`
  type to/from a `Value` tree directly, without going through JSON text
  (mirrors `serde_json::to_value`/`from_value`). Backed by a new
  `crate::value::ValueSerializer`, the serialize-side counterpart to the
  existing `ValueDeserializer`.
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
