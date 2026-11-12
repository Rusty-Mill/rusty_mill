---
category: Added
changelog: Mill Board ADR-0007 proposes the board's own SQLite store, the migration off rusty_tick and its rollback (design only).
---
## 2026-11-12 - Mill Board own-store design

- **Added:** ADR-0007 (Proposed), the design for card #1438 and Millwright B2. It covers a `SqliteStore` adapter behind the existing `Store` port on the workspace's `rusqlite`, the schema, one-transaction commits, a durable event log, the migration off rusty_tick with a zero-diff dry run, rollback, and removing the tick adapter and archiving rusty_tick. No code in this change.
- **Changed:** ADR-0012 points to its successor. Millwright records Nano's go to start the design (M3).
