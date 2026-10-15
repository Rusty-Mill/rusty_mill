---
category: Added
changelog: **Tests pin the BBP event order around the turn that crosses the turns budget.** Tests only; no engine change.
---
## 2026-10-15 - BBP core: event order at the turns-budget crossing (tests only)

- **Added (tests):** `rusty_bbp/tests/budget_order.rs` pins, event by event, what one command writes when the grant it causes crosses `budget.turns`: an accepted command (`TurnGranted, Escalated, TurnEnded, StateChanged, OpAccepted`), a refused command and a deadline regrant (`TurnGranted, Escalated, TurnEnded, StateChanged, Rejected`). Each asserts replay equals live and exactly one escalation.
- **Added (tests):** a log in the pre-#657 order (`TurnGranted, OpAccepted, Escalated, ..`, built by hand) folds, under schemas 2, 3 and 4, to the state the new order folds to except three rev stamps (`last_turn_end[planner]`, `ready_rev`, `entered_rev`), each exactly one later; and `wake_set` is equal at every prefix of the two logs.
- **Unchanged:** the engine and every log byte.
- **Known limitations:** the prefixes where the two logs differ all have an empty `wake_set` (a live turn, or the task escalated), so the differential covers the crossing window and not a `wake_set` after a resume. The old order was confirmed from real runs of the pre-#657 engine for the accepted, deadline and refused-read paths.
