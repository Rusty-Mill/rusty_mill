# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
- `request_framing`/`response_framing` now reject conflicting repeated
  `Content-Length` headers instead of silently framing on only the first
  occurrence (a request-smuggling-adjacent risk); identical repeats are
  still accepted.
- `ChunkedDecoder::advance` now enforces `max_line_len` on a complete
  chunk-size/trailer line delivered in one buffer, not only on a line split
  across reads — the limit no longer depends on input fragmentation.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
