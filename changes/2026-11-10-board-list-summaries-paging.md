---
category: Added
changelog: Mill Board `board_list` gains a summary view and cursor paging; a project-filtered summary is 92% smaller and reads a third of the storage.
---
## 2026-11-10 - Mill Board list summaries and cursor paging

- **Added:** `board_list` `view: "summary"` (task fields only: id, number, rev, title, project, column, priority, claim, tags and dates). It reads rusty_tick's tasks without board documents or comments: 3 requests and 1.24 MB instead of 7 and 3.69 MB on the 1,425-card profile; a `p03` summary payload is 12 KB instead of 161 KB.
- **Added:** `limit` (1–500) and `cursor` on `board_list`. Pages follow `(created_ms, id)` and stop at 512 KiB. The cursor is stateless, bound to its query and replayable after a restart; scope and project filters apply to every page, so a cursor carries no authority.
- **Unchanged:** without `view`, `limit` or `cursor`, `board_list` returns what it did. A full-view page still reads the whole board and pages only the payload.
