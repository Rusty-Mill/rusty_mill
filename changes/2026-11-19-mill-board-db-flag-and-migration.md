---
category: Added
changelog: Mill Board can keep its cards in its own SQLite file with --db, and --migrate-from-tick copies rusty_tick into it with a dry run that writes nothing (ADR-0007).
---
## 2026-11-19 - Mill Board --db and the tick-to-SQLite migration

- **Added:** `--db FILE` (`MILL_BOARD_DB`) runs the board on `SqliteStore`; `--db` with `--tick-url` is refused. Without `--db` nothing changes.
- **Added:** `--migrate-from-tick [--dry-run]` reads rusty_tick (read-only), loads an empty `--db` in one transaction, reads it back through the ports and reports counts and every mismatch (non-zero exit on any). `--dry-run` copies into a private in-memory database.
- **Known limitations:** rusty_tick's move history has no column, so migrated moves carry the card's current column; unknown board-document fields are not carried; events start empty (one resync). The production migration, removing tick and the deploy are separate decisions.
