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
- `Value::insert`/`remove`: mutate a `Value::Map` in place (overwrite-or-append
  and delete-by-key, matching `HashMap`/`serde_json::Map`'s conventions - a
  no-op returning `None` on a non-`Map` value, same as `Value::get`). Closes
  the gap that previously forced hand-rolled find-or-push logic anywhere a
  `Value` was used as a mutable document builder.
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
