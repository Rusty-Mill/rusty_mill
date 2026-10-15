---
category: Changed
changelog: **BBP wake rules revised after the shadow comparison.** `wake_set` ranks Addressed > State > Artifact > Observer, baselines a never-run role at its assignment, and records an artifact watcher that does not serve the state as an unselectable observer. Shadow-only; no grant changes.
---
## 2026-10-14 - BBP: wake rules after the shadow comparison

- **Changed:** `wake_set` (informational; nothing grants from it). Priority is now `Addressed` > `State` > `Artifact` > `Observer`, ties by `turn_order`. A role with no turn yet is baselined at its assignment rev instead of 0; artifact wakes are floored at the current state's entry rev; `Addressed` still uses the end of the last turn. `WakeReason` gains `Observer` (an artifact wake of a role that does not serve the state) and `WakeReason::selectable(max_live_turns)`: an observer is selectable only above one live turn. `Shadow::comparison()` compares the first selectable entry, and a set of observers only is a `Fallback`.
- **Added:** `TaskState.entered_rev` and `TaskState.assigned_rev`, fold-derived and not logged, so no log format change.
- **Changed (Addressed):** only a named recipient is `Addressed`; a message to `all` addresses no one for scheduling (visibility and read scope are unchanged).
- **Result:** over the 31 traces the automatic grants go from 26 agree / 2 fallback / 25 disagree to 50 / 3 / 0. The 3 fallbacks are a cooled-down holder and an observer-only set. On these scripted runs the rule now matches turn-based everywhere; the targeted cases show where it differs.
- **Added:** `WAKE_RULES` (now 2) and `Shadow.rules`: every shadow record says which wake rules computed it, and a record written before the field reads as 1. `bbp shadow` reports `by_rules` (rules, then state) next to the plain totals, so evidence from different rules is never summed. Bump `WAKE_RULES` whenever a trigger, the priority, a baseline or the addressing rule changes.
- **Added (tests):** five trace cases (idle default holder, a question to a non-default role, an artifact that leaves only observers, a broadcast verdict that now agrees as a regression guard, and a verdict naming a recipient that is still Addressed).
- **Known limitations:** `WakeReason` gains a variant and reorders, so a `match` outside this crate must be updated (none in the workspace). No event-driven grant exists.
