---
category: Added
changelog: BBP slot admission queues waiters fairly, by rank with priority aging, under a per-goal agent ceiling and lease write conflicts (Stage 3d).
---
## 2026-10-25 - Fair slot admission

- **Added:** `bbp mod --claim FILE`. Moderators waiting for a slot hold ranked tickets in the slot directory. A ticket's rank is its arrival time less `priority_secs`, so no waiter starves. A ticket that cannot run yet leaves the free slot to the next ticket. For agent slots, a parent's tasks share its ceiling and each held slot counts once. Two tasks whose lease write paths overlap never run harnesses at once, and a waiting task keeps its paths against later tickets. ADR-0018.
- **Added:** orch-bbp hosting `slots.ceiling` and `slots.priority_secs`. Every task of a goal claims the goal as its parent and claims the write paths it was frozen under.
- **Tests:** in-process tests of order, priority aging, the ceiling, lease conflicts, dead waiters and claim validation. A real-moderator test runs three tasks of one parent under a ceiling of two, beside another parent's task, in three slots. An orch-bbp test checks the claim a leased task is given.
- **Known limitations:** the ceiling covers agent slots only, and there is one level, goal over task. A reserved slot can sit idle for up to one poll. Read leases never conflict.
