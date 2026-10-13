---
category: Changed
changelog: **BBP event-driven note reconciled with the merged v0.5 engine.** Triggers rewritten against the real requirement table, T-final dropped, explicit grants separated from triggers, `pick_next` falls back to the default holder, shadow output is returned not logged, and the phase plan is split into one PR per step.
---
## 2026-10-13 - BBP event-driven note: reconciled with v0.5

- **Changed:** `docs/bbp-event-driven-design.md` now cites the merged engine (symbols and `main` line numbers), uses `wake_set(&TaskState)`, defines the three state-derived triggers and their order (`Addressed` > `Artifact` > `State`, ties by `turn_order`), documents the fold-derived watermark fields, adds the liveness fallback to `pick_next`, specifies shadow mode (`Handled.shadow`, not logged, host logging deduped on `rev`) and replaces the phase table with one PR per step.
- **Removed from the design:** T-final (redundant with the `review` row), and T-consult and T-reply as triggers (they are explicit grants that bypass `schedule()`).
- **Known limitations:** design only. The `seen_rev` baseline for a role with no turn yet is left open for the shadow comparison.
