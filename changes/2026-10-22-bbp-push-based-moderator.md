---
category: Changed
changelog: `bbp mod` reacts to events: a task-log append or a harness or runner exit wakes it at once, and `--poll-ms` is only the longest wait.
---
## 2026-10-22 - BBP push-based moderator

- **Changed:** `bbp mod` no longer sleeps a fixed poll between passes. It waits on the task's log (inotify) and on the harnesses and the runner it started (pidfds), and runs a pass as soon as one changes. Each wait also ends when the nearest turn or request deadline, the wall limit or an end grace comes due (`TaskState::next_deadline` in `rusty_bbp` names the deadlines a tick would fire), so a silent harness cannot hold them back; `--poll-ms` (default 500 ms) is only the longest wait. Step 1 of `docs/bbp-event-based-plan.md`; no protocol change.
- **Known limitations:** Linux only; where pidfds are unsupported or denied the children are checked every 100 ms instead (reported on stderr); elsewhere, or if the log cannot be watched (reported on stderr), the moderator sleeps the whole poll as before. Harness output is not watched, so the cost cap still lags by up to one poll.
