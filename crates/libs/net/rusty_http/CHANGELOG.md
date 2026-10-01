# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
- Transfer-Encoding framing no longer reads only the first field (design review 3.4).
  - Every `Transfer-Encoding` field is combined into one coding list.
  - `chunked` anywhere but once and last is refused, as is an empty element.
  - A request with both `Transfer-Encoding` and `Content-Length`, or whose coding does not end in `chunked`, is refused. These used to fall back to `Content-Length` framing, a request-smuggling ambiguity.
  - A response's non-chunked coding is read to EOF.
  - Errors are `Error::InvalidHeader`.
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
