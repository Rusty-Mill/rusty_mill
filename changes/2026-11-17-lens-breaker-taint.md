---
category: Added
changelog: `rusty_orch lens run` runs the #598 cost breaker, pauses a task on a ceiling or an enforced trip, and taints each post with what its provider could see (Let Em Loose #758).
---
## 2026-11-17 - Cost breaker and taint in lens runs

- **Added:** `lens run` judges each lens's fresh cards against the cost breaker's window (40 calls or $5, a score move of 4, from the lens config). `observe`, the default, logs `breaker would trip`; `enforce` runs `bbp pause --reason breaker` and ends the run. The window is kept in memory and starts again with each run (598 PLAN S8).
- **Added:** a task ceiling that halts a round now runs `bbp pause --reason spend_ceiling` or `call_ceiling`. Pause ops are `orch-<run>-<reason>-<calls>`, so a new run after a resume pauses again.
- **Added:** every post that lands is tainted with the highest class its provider's context admitted (`round::Report::taints`). The table is saved per board before the scheduler state and applied to every card and settled line each pass, and to the alien lens's cards, so a tainted claim is shown only to providers cleared for its class.
