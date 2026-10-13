---
category: Added
changelog: **BBP scheduling seam.** `task_opened` can record a `SchedulingPolicy` (only `TurnBased`, which is not written), and `schedule()` picks who takes the next turns through `pick_next`. Behaviour and turn-based logs are unchanged.
---
## 2026-10-13 - BBP: scheduling policy and `pick_next` seam

- **Added:** `SchedulingPolicy { TurnBased }` (default), an additive `policy` field on `task_opened` and `OpenTask`. `TurnBased` is skipped when serialized and absent reads as `TurnBased`, so a turn-based log is byte for byte the schema 2 log (the golden in `tests/schema.rs` is untouched). `engine::pick_next(&TaskState) -> Vec<RoleId>` (crate-private) returns the roles that serve the state in `turn_order`; `schedule()` grants from it and `grant` still admits at most `max_live_turns`.
- **Changed:** `OpenTask` gains a `policy` field, so a caller that builds one by struct literal must set it (`SchedulingPolicy::default()`); the host's `admin` does. A payload without it decodes as `TurnBased`.
- **Unchanged:** the 31 traces, the proptest model, the schema-2 golden and every pre-existing test pass; explicit grants still bypass `pick_next`.
- **Known limitations:** nothing selects any policy but `TurnBased`, and `wake_set` is not yet consulted. Shadow mode (step 4) compares them next.
