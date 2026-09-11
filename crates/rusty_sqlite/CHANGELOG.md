# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
### Fixed
- `PooledConnection::drop` now rolls back an open transaction (or discards the
  connection if rollback fails) before returning it to the pool, instead of
  recycling a connection with uncommitted state visible to the next borrower.
- `build_pool_with_timeout` now rejects `max_size == 0` immediately instead of
  constructing a pool that can never satisfy an acquire.
- `Migrations::validate_order` now rejects any non-positive migration version
  individually, not just pairwise ordering — a version-0 migration was
  previously accepted by validation and then silently skipped by `run`.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
