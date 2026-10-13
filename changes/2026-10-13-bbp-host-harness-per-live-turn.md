---
category: Added
changelog: **BBP host runs one harness per live turn.** `bbp open --live-turns N` opens a task with up to N concurrent turns; the moderator supervises a harness for each, kills the ones a transition revokes, and aborts only the turn of a harness that died.
---
## 2026-10-13 - BBP host: one harness per live turn

- **Added:** `admin::open_task_live` and `bbp open --live-turns N` (default 1, same as before). The moderator holds a list of harnesses instead of one; `bbp mcp` treats its turn as live while it is any of the live turns, not only the first.
- **Test:** two builders at `max_live_turns = 2` run in parallel (the coder's harness waits for the other builder's to start, so it deadlocks if serialized); the coder's candidate revokes the other turn and its harness process is killed. Fails by timeout when the cap is 1.
- **Known limitations:** no throughput measurement yet (metrics follow); a harness that exits is forfeited one at a time per loop iteration.
