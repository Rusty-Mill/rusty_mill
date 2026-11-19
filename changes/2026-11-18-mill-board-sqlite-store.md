---
category: Added
changelog: Mill Board gains SqliteStore, its own SQLite store behind the storage port (ADR-0007); not wired into the binary.
---
## 2026-11-18 - Mill Board SqliteStore (ADR-0007, card #1438)

- **Added:** `SqliteStore` implements `Store` and `MilestoneStore` on the workspace's `rusty_sqlite` wrapper: one transaction per commit (card and event together), a durable event log and card numbers, versioned schema by `user_version`, and indexed queries for numbers, columns, open cards and moves. It passes the shared store contract scenario, and its answers are tested against `MemStore`.
- **Known limitations:** not wired into the binary (no `--db` flag), no migration from rusty_tick, no tick removal; each is a separate, authorized step in ADR-0007. A card is stored whole as JSON beside indexed columns, so comments and tags are not separate tables yet, and events carry no separate `actor`/`kind`/`rev` columns (the full event is in its JSON doc); owner accepted these departures from the ADR (Nano, 2026-11-19). Moves are their own `moves` table, so they survive event pruning. One connection behind a mutex, no read pool. No event pruning yet. Scale ceilings are not measured.
