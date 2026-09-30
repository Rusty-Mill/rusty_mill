# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
### Changed
- `Database::snapshot` returns an opaque `Snapshot` (tables and index metadata) instead of `HashMap<String, Table>`; `Database::restore` takes it.
### Fixed
- A rolled-back transaction reached the database file: every `execute` wrote the whole file, even inside a transaction. The file is now written only outside a transaction and at the outermost commit.
- Rolling back did not undo `CREATE INDEX`/`DROP INDEX`.
- `flush` overwrote the file in place, so an interrupted write could destroy the previous image. It now writes a synced temporary file, renames it over the database, and syncs the directory (Unix).
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
