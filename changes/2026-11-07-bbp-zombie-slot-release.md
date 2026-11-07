---
category: Fixed
changelog: BBP slot holders release after cancellation when only unreaped zombie members remain.
---
## 2026-11-07 - Release BBP slots after zombie-only cancellation

- **Fixed:** a dead moderator's cancelled harness could leave a slot holder waiting for zombie group members that cannot run. Confirmed zombies no longer hold the slot; live members and unreadable or unknown process state still do. The existing double scan and pidfd cancellation fencing remain intact.
- **Tests:** real-process zombie-only and mixed live/zombie regressions, conservative process-scan fixtures, and existing moderator-loss cancellation acceptance cases.
- **Known limitations:** confined lifecycle acceptance still requires a Linux host enforcing Landlock. This fix does not establish deployed dispatcher or Codex Cloud recovery readiness.
