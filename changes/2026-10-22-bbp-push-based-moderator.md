---
category: Changed
changelog: `bbp mod` reacts to events: a task-log append or a harness or runner exit wakes it at once, and `--poll-ms` is only the longest wait.
---
## 2026-10-22 - BBP push-based moderator

- **Changed:** `bbp mod` no longer sleeps a fixed poll between passes. It waits on the task's log (inotify) and on the harnesses and the runner it started (pidfds), and runs a pass as soon as one changes. `--poll-ms` (default 500 ms) is now the longest wait, so turn deadlines, the wall limit, the end grace and the cost cap are still checked at least that often. Step 1 of `docs/bbp-event-based-plan.md`; no protocol change.
- **Known limitations:** Linux only; elsewhere, or if the log cannot be watched (reported on stderr), the moderator sleeps the whole poll as before. Harness output is not watched, so the cost cap still lags by up to one poll.
