---
category: Added
changelog: **BBP scheduling seam.** `task_opened` records a `SchedulingPolicy` (only `TurnBased`), and `schedule()` picks the next turn holder through `pick_next`, which returns a set capped at one by the policy. Behaviour is unchanged.
---
## 2026-10-13 - BBP: scheduling policy and `pick_next` seam

- **Added:** `SchedulingPolicy { TurnBased }` (default), recorded as an additive `policy` field on `task_opened` and on `OpenTask`. `engine::pick_next(&TaskState) -> Vec<RoleId>` returns who may take the next turn; `SchedulingPolicy::max_live` caps the set (one). `schedule()` grants the first of it. For `TurnBased` the set is the default holder, so grants are identical to before.
- **Changed:** `OpenTask` gains a `policy` field, so a caller that builds one by struct literal must set it (`SchedulingPolicy::default()`); the host's `admin` does. The wire format is additive: a log without `policy` still decodes, as `TurnBased`, with the schema still 2. Logs written from now on carry `"policy":"TurnBased"`.
- **Unchanged:** the 31 traces, the proptest model and every pre-existing test pass without edits to their assertions; explicit grants still bypass `pick_next`.
- **Known limitations:** nothing selects any policy but `TurnBased`, and `wake_set` is not yet consulted. Shadow mode (step 4) compares them next.
