---
category: Added
changelog: **Tests pin the BBP event order around the turn that crosses the turns budget.** Tests only; no engine change.
---
## 2026-10-15 - BBP core: event order at the turns-budget crossing (tests only)

- **Added (tests):** `rusty_bbp/tests/budget_order.rs` pins, event by event, what one command writes when the grant it causes crosses `budget.turns`: an accepted command (`TurnGranted, Escalated, TurnEnded, StateChanged, OpAccepted`), a refused command and a deadline regrant (`TurnGranted, Escalated, TurnEnded, StateChanged, Rejected`). Each asserts replay equals live and exactly one escalation.
- **Finding, not tested:** since #657 `grant` escalates immediately after `TurnGranted`; before it, an accepted command wrote `TurnGranted, OpAccepted, Escalated`, and a refused command or deadline regrant did not escalate until a later command. A log in the old order folds without error under schemas 2, 3 and 4, but not to an identical `TaskState`: `last_turn_end`, `ready_rev` and `entered_rev` are rev stamps and differ by one. Everything else is equal. No engine change here.
- **Unchanged:** the engine and every log byte.
- **Known limitations:** the old order was confirmed from a real run of the pre-#657 engine for the accepted and deadline paths; the refused path was not run on the old engine.
