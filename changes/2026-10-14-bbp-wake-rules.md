---
category: Changed
changelog: **BBP wake rules revised after the shadow comparison.** `wake_set` ranks Addressed > State > Artifact > Observer, baselines a never-run role at its assignment, and records an artifact watcher that does not serve the state as an unselectable observer. Shadow-only; no grant changes.
---
## 2026-10-14 - BBP: wake rules after the shadow comparison

- **Changed:** `wake_set` (informational; nothing grants from it). Priority is now `Addressed` > `State` > `Artifact` > `Observer`, ties by `turn_order`. A role with no turn yet is baselined at its assignment rev instead of 0; artifact wakes are floored at the current state's entry rev; `Addressed` still uses the end of the last turn. `WakeReason` gains `Observer` (an artifact wake of a role that does not serve the state) and `WakeReason::selectable(max_live_turns)`: an observer is selectable only above one live turn. `Shadow::comparison()` compares the first selectable entry, and a set of observers only is a `Fallback`.
- **Added:** `TaskState.entered_rev` and `TaskState.assigned_rev`, fold-derived and not logged, so no log format change.
- **Result:** over the 31 traces the automatic grants go from 26 agree / 2 fallback / 25 disagree to 39 / 3 / 11. All 11 remaining come from a message addressed to `all` addressing every role that can act; that is an open question, not changed here.
- **Added (tests):** four trace cases where turn-based wastes a turn (idle default holder, a question to a non-default role, an artifact that leaves only observers, and the broadcast verdict that still diverges).
- **Known limitations:** `WakeReason` gains a variant and reorders, so a `match` outside this crate must be updated (none in the workspace). No event-driven grant exists.
