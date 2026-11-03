---
category: Changed
changelog: **`rusty_bbp_host`: inbox order note.** The README records that cross-task age ordering in `bbp inbox` is deferred until more than one claims task escalates at a time.
---
## 2026-11-02 - rusty_bbp_host: inbox order note (Let Em Loose #595)

- **Changed:** `bbp inbox` keeps its per-task order (irreversible first, then tasks in the order listed, oldest first within each). Ordering by age across tasks, which needs an escalation time in the log, is deferred until more than one claims task escalates at a time (owner decision, 2026-11-02).
