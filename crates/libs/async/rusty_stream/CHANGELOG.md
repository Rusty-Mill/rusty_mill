# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
- `Segment::open_on` recovery now reads a full record's declared length
  (issuing follow-up reads as needed) before deciding EOF-mid-record means
  "torn", instead of assuming a record fits in the first 64 KiB read — a
  complete record over 64 KiB, or a genuinely short read, was previously
  misclassified as torn and truncated along with everything after it.
- `Segment::create_on`/`Segment::append` now verify the full header/record
  was actually written (looping via the underlying write-all fix in
  `rusty_tokio`) before advancing `write_pos`/publishing an index entry,
  instead of treating any successful short write as a complete record.
- `ConsumerOffsets::commit` now validates a consumer ID's UTF-8 byte length
  before encoding, instead of silently truncating IDs over 65535 bytes into
  an unrecoverable offset-store record.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
