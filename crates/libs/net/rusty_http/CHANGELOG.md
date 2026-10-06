# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
- `head::parse_request_head`/`parse_response_head` now enforce
  `max_head_len` on a head that *completes* as well as on one that has
  not: a terminated head larger than the cap is `Error::HeadTooLarge`
  instead of being accepted. Only the head's own bytes (`consumed`) are
  counted, so body or upgrade bytes already buffered after the blank line
  never trip it and the byte-exact upgrade contract is unchanged.
- `read_body` on all three transport adapters (`sync`, `async_tokio`,
  `tokio_native`) now bounds a chunked body's decoded total by
  `DEFAULT_MAX_BODY_LEN`, as it already did for `Content-Length` and
  close-delimited framing; the bound is a checked addition before each
  extend. `read_chunked_body` stays a line-bounded primitive with no
  aggregate cap and says so in its docs; the sync `read_request_body`
  chunked path keeps its cap and uses the same checked arithmetic.
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
