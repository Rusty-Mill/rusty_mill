---
category: Added
changelog: "rusty_tick: durable board column history (`GET /api/v1/moves`, `GET /api/v1/tasks/{id}/moves`), recorded on every column change with an optional `X-Actor`, for Mill Board's time-in-column metrics."
---
## 2026-10-30 - Board column history in rusty_tick

- **Why:** Mill Board measures time in each column, cycle time and blocked time, but its move events lived only in its process memory and were lost on restart.
- **Added (`rusty_tick`):** an append-only `moves.mmap` table. A task's column is read exactly as Mill Board's `card_of` reads it: `null` without the `agent` tag, `done` when closed, else its first `board:<col>` tag if that names a board column, else `backlog`. Every task write that changes it (a board `PATCH`, a create, complete or reopen, an edit in tick's own UI, a tag rename) appends `{seq, taskId, from, to, atMs, actor}` in the same locked request, at the write's `updatedMs`. The actor is the request's `X-Actor` header (1 to 64 printable ASCII characters), or `tick`.
- **Added (`rusty_tick`):** `GET /api/v1/moves?since=<ms>&after=<seq>&limit=<n>` (oldest first, paged by `seq`, at most 1000 a page) and `GET /api/v1/tasks/{id}/moves`.
- **Design:** a dedicated table rather than a document kind, so history is typed, indexed by task and cannot be written through `/docs`. Nothing is deleted, a purged task's history included, so a `seq` cursor is never reused. A failed history append refuses further task writes until the store is reopened (as the task store does after an ambiguous failure), so a later move cannot hide the gap; on open, a task whose last recorded column differs from its column gets the missing move. A history log whose creation was interrupted (empty or a header prefix) holds no move and is cleared on open.
- **Known limitations:** no backfill; history starts at deploy, and a task with no history is left alone by the repair, so a failure on a card's very first move loses that one entry (the board falls back to `createdMs`). `X-Actor` is attribution the caller asserts, not authentication.
