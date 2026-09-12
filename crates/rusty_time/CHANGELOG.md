# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
- `Time::from_hms_nano` now rejects `nano >= 1_000_000_000` instead of accepting
  any `u32`, matching the documented range of `nanosecond()`.
- `DateTime::to_iso8601` now emits the stored UTC offset and fractional seconds
  instead of always appending a literal `Z`, which previously changed the
  represented instant and dropped sub-second precision on round-trip.
- `DateTime::parse` now validates offset hours (0-23) and minutes (0-59)
  before converting them to seconds, instead of accepting components like
  `+00:99`/`+99:00`.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
