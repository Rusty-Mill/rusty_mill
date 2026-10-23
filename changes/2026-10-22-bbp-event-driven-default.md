---
category: Changed
changelog: `bbp open` schedules one-live-turn tasks event-driven by default; `--policy turn-based` opts out.
---
## 2026-10-22 - BBP event-driven scheduling by default

- **Changed:** a task opened without `--policy` (by `bbp open`, `admin::open_task` or Orch's BBP host) with one live turn is now `EventDriven`: the next turn goes to the first role `wake_set` makes selectable, falling back to the turn-based holder when none is. Tasks with `--live-turns` above 1 stay `TurnBased`, the only policy that allows several live turns. Step 2 of `docs/bbp-event-based-plan.md`.
- **Unchanged:** replay. A log without the policy field, or below schema 4, still folds as `TurnBased`, which stays `SchedulingPolicy::default()`; `--policy turn-based` keeps the old schedule for new tasks.
- **Known limitations:** an event-driven task's log is schema 4, which a `bbp` built before the policy field refuses to replay. Validated with scripted agents (the moderator end-to-end runs and the 13 Orch confined acceptance cases); no live-model run.
