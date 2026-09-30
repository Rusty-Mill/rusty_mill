# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Fixed
- `Timeout`, `WaitJob` and `PidfdReady` wake the task that polled them most recently (design review 4). They used to wake the task from their *first* poll, so a future polled again from another task or executor never learned it was ready.
### Added
- `platform_async::waker_slot::WakerSlot`, a replaceable shared waker used by those futures.
- `EpollReactor::update_waker`.
### Added
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
