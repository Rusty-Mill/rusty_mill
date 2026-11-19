---
category: Added
changelog: Mill Board can keep its cards in its own SQLite file with --db, and --migrate-from-tick copies rusty_tick into it with a dry run that writes nothing (ADR-0007).
---
## 2026-11-19 - Mill Board --db and the tick-to-SQLite migration

- **Added:** `--db FILE` (`MILL_BOARD_DB`) runs the board on `SqliteStore`; `--db` with `--tick-url` is refused. Without `--db` nothing changes.
- **Added:** `--migrate-from-tick [--dry-run]` reads rusty_tick (read-only), builds a new `--db` file in a staging file of its own, reads it back (every card, milestone, reservation and move with its time, and what the source held beyond the card: unknown board-document fields and comment ids, carried under their own key) and reports counts and every mismatch (non-zero exit on any). The file is published by hard link only if the copy is exact and no existing file or `-wal`/`-shm`/`-journal` sidecar is in the way; an incomplete move history, a malformed reservation or an unreadable document is an error. `--dry-run` copies into a private in-memory database, so nothing is written.
- **Known limitations:** rusty_tick's move history has no column, so migrated moves carry the card's current column; events start empty (one resync). The production migration, removing tick and the deploy are separate decisions.
