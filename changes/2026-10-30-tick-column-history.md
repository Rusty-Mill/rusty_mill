---
category: Added
changelog: "rusty_tick: durable board column history (`GET /api/v1/moves`, `GET /api/v1/tasks/{id}/moves`), recorded on every column change with an optional `X-Actor`, for Mill Board's time-in-column metrics."
---
## 2026-10-30 - Board column history in rusty_tick

- **Why:** Mill Board measures time in each column, cycle time and blocked time, but its move events lived only in its process memory and were lost on restart.
- **Added (`rusty_tick`):** an append-only `moves.mmap` table. A task's column is its `board:<col>` tag, `done` when it is closed, `null` without one. Every task write that changes it (a board `PATCH`, a create, complete or reopen, an edit in tick's own UI, a tag rename) appends `{seq, taskId, from, to, atMs, actor}` in the same locked request, at the write's `updatedMs`. The actor is the request's `X-Actor` header (1 to 64 printable ASCII characters), or `tick`.
- **Added (`rusty_tick`):** `GET /api/v1/moves?since=<ms>&after=<seq>&limit=<n>` (oldest first, paged by `seq`, at most 1000 a page) and `GET /api/v1/tasks/{id}/moves`.
- **Design:** a dedicated table rather than a document kind, so history is typed, indexed by task and cannot be written through `/docs`. Everything is kept; a purged task's history goes with it, like its comments. A crash between a task write and its move is repaired on open: a task whose last recorded column differs from its column gets the missing move.
- **Known limitations:** no backfill; history starts at deploy, and a task with no history is left alone by the repair. `X-Actor` is attribution the caller asserts, not authentication.
