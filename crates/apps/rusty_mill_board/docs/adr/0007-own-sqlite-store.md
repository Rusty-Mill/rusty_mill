# ADR-0007: Mill Board owns its store (SQLite)

- **Status:** Proposed (2026-11-12). Nano asked for this design note: decision card in the Mill_Board thread, 2026-11-12, "Write the #1438 design note". It is card #1438 and Millwright milestone B2. Nothing here is built until Nano approves this note. The production migration, deleting the tick adapter and archiving rusty_tick are each authorized separately.
- **Supersedes:** [ADR-0012](../../../../../docs/adr/0012-one-task-store.md) (rusty_tick as the one task store), per Nano's 2026-11-10 decision (Millwright `board-store`, remind-me `mem_c6d02995042b4dedbb6a18e84730853f`).

## Context
Under ADR-0012, cards are rusty_tick tasks tagged `agent`. Each card also has a `board` document, a set of `comment` documents and a `board_number` reservation, and milestones are tasks tagged `milestone`. Nano does not use rusty_tick for anything else, so the board pays the cost of being a view over another app and gets nothing for it:
- Every unfiltered read pulls 4.5 MB from tick and filters it in the board. Bounded reads (#1426, S0–S7) shrank what leaves the board, but not what the board reads.
- One card write takes up to three fenced HTTP steps (`PartlySaved`, `card_plan`).
- The event log lives only in memory, so a restart forces every poller to resync (S2).
- Outside edits made in tick have to be noticed by polling (`observe`).

The `Store` port (`src/store.rs`) already isolates all of this. `MemStore` and `TickStore` are its two adapters, and `tests/store_contract.rs` runs one scenario against both.

## Decision
Add a third adapter, `SqliteStore` (`src/sqlite.rs`), on the workspace's existing `rusqlite` 0.41 (`bundled`, `backup`). It adds no new dependency. The board becomes the only writer of its own file. Once the migration is verified, `TickStore` and its fake are deleted and rusty_tick is archived.

### Schema
There is one file, `--db PATH`, defaulting to `~/.mill-board/board.db`. It uses WAL mode with `foreign_keys=ON`, and `user_version` holds the schema version, with migrations applied at startup in order.

| Table | Columns (keys) | Notes |
|---|---|---|
| `cards` | `id` TEXT PK, `number` INTEGER UNIQUE NULL, `rev`, `title`, `body`, `prompt`, `column`, `claim`, `project`, `priority`, `sort_order`, `issue`, `synced_text`, `created_ms`, `updated_ms`, `done_ms`, `archived_ms`, `restored_ms`, `target_ms`, `doc` TEXT (JSON) | `doc` holds the card's links and the rest as JSON: `deps`, `superseded_by`, `related`, `parent`, `milestone`, `execution` and `external_refs`, plus unknown fields kept per S5a. That way a new field needs no migration, and the S5a rollback guarantee carries over. Indexes: `(column)`, `(project, column)`, `(updated_ms)`, `(created_ms, id)`. |
| `tags` | `card_id`, `tag` (PK both) | User tags only. Column, claim, number and archive state become real columns instead of reserved tags. |
| `comments` | `id` TEXT PK (UUIDv7), `card_id`, `by`, `at_ms`, `text` | Index `(card_id, at_ms)`. Ids are kept from tick. |
| `events` | `id` TEXT PK (UUIDv7), `card_id`, `actor`, `kind`, `rev`, `column`, `at_ms` | **Durable.** A restart no longer forces a resync. It keeps the last 100,000 events and prunes older ones at startup, and a cursor older than the pruned point gets `resync: true` as today. |
| `moves` | `card_id`, `at_ms`, `from`, `to` | Replaces tick's `GET /moves`, which `moved_in` reads. It is written in the same transaction as the move. |
| `numbers` | `number` PK, `card_id` | Reservations are never reused, as with tick's `board_number` docs (#1238). |
| `milestones` | `id` PK, `project`, `title`, `target_ms`, `done_ms`, `rev`, … | This is the `MilestoneStore` port (`src/milestones.rs`). |

Settings stay in their files (`--settings-dir`, ADR-0004): they are config, not records.

### Commit semantics
- One `BEGIN IMMEDIATE` transaction per commit writes the card row, its tags, its comments delta, the move if any, and the event. The event id is assigned inside the transaction.
- `replace` checks `rev = card.rev - 1` in the `UPDATE … WHERE id=? AND rev=?`, and zero rows changed is `Conflict`. This is the same compare-and-swap contract as before.
- With one transaction per commit, `PartlySaved` can no longer happen on this adapter. The variant stays for the port.
- A card and its event become visible together, as with `MemStore`, which is a stronger promise than `TickStore` makes.
- One `Mutex<Connection>` handles writes, and reads go through a small pool of read-only connections. That is plain `rusqlite` without r2d2: the board is one process, and there are only a few writes per second.

### Reads
- `summaries`, `in_columns`, `numbered`, `open_among`, `changed_since` and `moved_in` each become a single indexed query, so none of them reads every card.
- The full `board_list` reads cards, tags and comments in three queries.
- The S2 observation machinery (`observe`, the comment window, the 5-minute reconcile) is no longer needed: the board is the only writer. Anyone who edits the file by hand must restart the board.

## Migration (B3, separately authorized)
`rusty_mill_board migrate-from-tick --tick-url … --db NEW.db [--dry-run]` works as follows:
1. **Export:** it reads through the existing `TickStore` read paths (tasks tagged `agent`, board docs, comments, `board_number` docs, milestones, `/moves`) and writes `NEW.db` in one transaction. It keeps ids, numbers, revs, `created_ms`/`updated_ms`, comment ids, authors and times, move history and unknown doc fields. Events are not migrated (tick keeps none), and the new log starts empty with a `resync` cursor.
2. **Verify:** it reads both stores through the `Store` port and compares them card by card, and every field must be equal. It also compares counts per table, the highest reserved number and a checksum of the serialized cards. Any difference fails the run.
3. **Dry run first:** the dry run runs against production tick and writes to `/tmp`. It changes nothing in tick. The result (counts and diff, expected to be zero) goes to Nano.
4. **Cutover:** stop the board, run the migration for real, start the board with `--db` instead of `--tick-url`, then check health and spot-check `card_get`, `board_list` and `board_events`. Tick stays running, read-only for the board, until archiving.

### Rollback
- **Before archiving:** stop the board and start the `.prev` binary with `--tick-url`. Writes made since the cutover exist only in SQLite. To keep them, `export-to-tick` replays cards changed after the cutover time into tick (same port, reverse direction).
- **After archiving:** rollback needs tick restarted from its read-only data. Archiving therefore waits at least 14 days after the cutover.

## Removal and archive (B3, separately authorized, after the rollback window)
- Delete `src/tick.rs`, `tests/fake_tick/`, the tick-only tests and the `--tick-url` flag. `MemStore` stays for tests, and `store_contract.rs` runs against `MemStore` and `SqliteStore`.
- Archive rusty_tick: stop and disable its service, keep the binary and data read-only, and mark the crate archived in its README. Nothing is deleted.
- Cards to re-plan: #1101 (superseded), #1100 (settings history moves to the `events` table or its own table), and #1433 (S4, superseded).

## Testing
- `tests/store_contract.rs` gains `SqliteStore` on a temp file and must pass unchanged.
- New tests:
  - a commit is atomic under a crash (the process is killed mid-transaction, and the file reopens consistent);
  - concurrent `replace` gives exactly one winner;
  - events survive a reopen;
  - unknown doc fields survive a write.
- The migration is tested against `FakeTick`, seeded with every field set and two of each relation, with zero diff. Each comparison is checked to fail on a corrupted copy.
- The S6 scale harness gains a SQLite row. Target ceilings: `board_list` full at 1,425 cards ≤ 50 ms, summary ≤ 10 ms, `card_take` ≤ 5 ms. These are set after the first measurement, not before.

## Consequences
- Reads and writes become local and indexed, and one commit is one transaction.
- The event log survives restarts, and the S2 observation code is no longer needed.
- Backups: `rusqlite`'s `backup` API serves a nightly `board.db.bak` (added to the deploy prompt) and is used before every migration.
- Cards can no longer be edited from rusty_tick's UI. That is intended, since Nano doesn't use it.
- Rejected alternatives:
  - **Keep tick and add filters (S4):** it treats the symptom and keeps two apps.
  - **remind-me's store, or an embedded KV:** neither has a relational fit for cards, comments and moves, and both would add a dependency.
  - **A schema of one JSON document per card:** it loses indexed column, project and number queries.
  - **Postgres:** it needs a service to run, which a single-process board doesn't need.

## Open questions for Nano
1. Is 14 days the right rollback window before archiving rusty_tick?
2. Should event retention be 100,000 events, or time-based, for example 90 days?
