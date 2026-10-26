---
category: Added
changelog: rusty_tick gains Mill Board's `board` document kind, single-document reads, document preconditions and comment authors (ADR-0012, step 1).
---
## 2026-10-26 - rusty_tick: board documents and document preconditions (ADR-0012 step 1)

- **Added:** a `board` document kind for Mill Board. Its id is its task's id, a task in the trash cannot get one, and purging the task (directly or by emptying the trash) deletes it, as it already deleted comments.
- **Added:** `GET /api/v1/docs/{kind}/{id}`. Every document carries an `etag` (its `updatedMs`, which now advances on every write even within one millisecond). `PUT` honours `If-Match` and `If-None-Match: *`. A `board` document's `PUT` also requires `If-Task-Match: <task etag>`, and both are checked under the request lock, so a write authorized against a task that has since changed is refused with 412.
- **Added:** comments show an optional `by` (who wrote it), for comments an agent posts through Mill Board.
- **Known limitations:** `DELETE` on a document does not take preconditions yet; nothing needs it.
