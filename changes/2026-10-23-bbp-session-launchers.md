---
category: Added
changelog: `bbp mod` can keep one harness per role across turns (`"session": true` in the agents file), woken for each turn by `wait`.
---
## 2026-10-23 - BBP session launchers

- **Added:** a launcher with `"session": true` runs one harness for its role across the task, bound to a session `bbp mcp`. The moderator starts it at the role's first grant and hands it each later grant through `wait`. It keeps running while the role's turns end by its own move; a turn ended from outside (deadline, budget, abort, revocation) stops it at once, cancelling inference in flight and its queued calls, and the next grant starts a fresh one. Step 3b of `docs/bbp-event-based-plan.md`.
- **Added:** `TaskState::turn_end_cause`, derived from the log (never written), so the host can tell an agent's own turn end from an external one.
- **Unchanged:** launchers without `session` (the default) still get one harness per turn.
- **Known limitations:** a session launcher is refused under a cost cap or with a `sandbox`, and may not use `{turn}`; `bbp tokens` reports session turns as unknown cost. Both come with 3c. Validated with scripted shell agents; no live-model run.
