# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
### Security
- Connection hardening (design review 3.5):
  - A frame other than CONTINUATION inside a pending header block is a connection error.
  - Header blocks are capped at 64 CONTINUATION frames and at `SETTINGS_MAX_HEADER_LIST_SIZE`, or 64 KiB if unset, never above 1 MiB.
  - DATA beyond the connection or stream receive window is a flow-control error.
- New `Connection::send_frame`: outgoing frames now use `Send*` stream transitions and the send windows. Before, they went through the receive path, which decoded our own HEADERS with the peer's HPACK state and charged DATA to the receive window.
- New `Connection::release_capacity`: returns consumed bytes as WINDOW_UPDATE frames. **Readers must call it** now that receive windows are enforced.
- The client's `send_request` uses `send_frame`.
- Removed `connect/flow.rs`, a second flow-control implementation that was never compiled and had a bug in its stream-window update.

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
