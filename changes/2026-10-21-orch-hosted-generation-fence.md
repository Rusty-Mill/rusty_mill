---
category: Added
changelog: Fence every hosted Orch command with a durable assignment generation and holder, refused by the BBP host once superseded.
---
## 2026-10-21 - Hosted commands carry a durable assignment fence

- **Added:** `orch-store` keeps each goal's orchestrator assignments, parent generations and execution links in a separate assignment record, rebuilt through the new `Factory::restore`, which re-checks ids, ancestry, budgets, scopes and link generations.
- **Added:** each `rusty_orch run` takes over the goal's `goal-orchestrator` assignment at its next generation (`Factory::take_over`) and saves it before any host command; `maintain` does the same in its journal.
- **Changed:** every `TaskHost` command carries a `Fence` (assignment, generation, per-process holder). The BBP host publishes each admitted generation once per task and refuses lower generations and other holders of the same one as unavailable, so a run from a stale copy of the state neither drives nor fails the task.
- **Known limitations:** the most recent process to start takes over; there is no wait for the earlier one to stop. The Millboard card link still has no Orch caller, and `TaskHost` has no cancel yet.
