# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `DateTime::from_unix_secs(i64) -> Result<DateTime, &str>`: the checked
  inverse of `DateTime::timestamp` (UTC, zero nanoseconds, zero offset;
  euclidean split so pre-1970 instants work). An instant whose civil year
  does not fit a `Date`'s `i32` year is an error, never wrapped or clamped.
  The crate stays `no_std` and clock-free: reading "now" is the caller's job.
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
