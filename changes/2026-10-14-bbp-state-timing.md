---
category: Added
changelog: **BBP observed state-visit timing.** `bbp mod` prints `state_ms`, the ordered wall-clock durations of state visits observed during one moderator invocation; these raw visits do not define authoring time for the ultimately approved spec.
---
## 2026-10-14 - BBP per-state wall timing

- **Added:** `moderator::Outcome.visits` (ordered `(State, Duration)`, an observed revisit is its own entry) and `state_ms` in the `bbp mod` JSON, e.g. `[["Planning",41200],["PlanGate",300],...]`.
- **Interpretation:** the first `Planning` visit excludes rework after a rejected plan. An authoring-time endpoint for the ultimately approved spec, including how to handle replanning, must be defined before using these observations for 2b criterion 3. This PR does not define that endpoint or new timing semantics.
- **Observation limits:** the log carries no timestamps; durations use the moderator's clock. Transitions between observations are invisible, including a departure and return to the same state. Loop work and scheduling delays are included, so error is not bounded by one configured poll interval (`--poll-ms`, 500 by default).
- **Lifetime and exits:** timing covers one moderator invocation; restarting starts a fresh series. Error and wall-limit exits return no `Outcome` and discard visits. On successful completion, visits end at the terminal-state observation before final harness cleanup, while `elapsed` includes that cleanup; their totals need not match.
- **Validation scope:** injected-instant tests check observed ordering, revisits and a rejected-plan/replanning sequence, not the integration limits above. Not an evidence run or approval for an experiment or live adoption.
