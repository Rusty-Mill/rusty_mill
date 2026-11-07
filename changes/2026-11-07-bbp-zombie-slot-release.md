---
category: Fixed
changelog: BBP slot holders release after cancellation when only unreaped zombie members remain.
---
## 2026-11-07 - Release BBP slots after zombie-only cancellation

- **Fixed:** a dead moderator's cancelled harness could leave a slot holder waiting for zombie group members that cannot run. Only a confirmed single-threaded zombie no longer holds the slot: a zombie main thread with a live worker still does, as do unreadable or unknown process/thread counts. The existing double scan and pidfd cancellation fencing remain intact.
- **Tests:** real-process zombie-only, mixed live/zombie and zombie-main/live-pthread regressions, conservative process-scan fixtures, and existing moderator-loss cancellation acceptance cases.
- **Known limitations:** confined lifecycle acceptance still requires a Linux host enforcing Landlock. This fix does not establish deployed dispatcher or Codex Cloud recovery readiness.
