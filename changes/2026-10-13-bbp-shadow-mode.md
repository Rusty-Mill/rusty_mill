---
category: Added
changelog: **BBP shadow mode.** `handle` returns, beside each real grant, what the event-driven `wake_set` would have woken; forced grants are labelled apart. Nothing is logged and no grant changes.
---
## 2026-10-13 - BBP: shadow mode

- **Added:** `Handled.shadow: Vec<Shadow>` and `Driver.shadow` (the last dispatch's). `Shadow { rev, state, kind }` with `ShadowKind::Auto { granted, would }` for a `schedule()` grant and `ShadowKind::Explicit { granted, kind }` for a yield, human `ask`, settlement or consultation return. `Shadow::comparison()` is `Agree`, `Fallback` (empty `wake_set`, which falls back to the default holder) or `Disagree`; `None` for explicit grants.
- **Unchanged:** the log, every grant, and every existing test (the 31 traces, the proptest model and the schema-2 golden are not edited). Recorded only for a turn-based task with one live turn; nothing is recorded above `max_live_turns` 1.
- **Changed (tests):** `Fx::run` checks after every command of every test that the shadow records match the `turn_granted` events, and prints the records when `BBP_SHADOW_TRACE` is set.
- **Known limitations:** `Handled` gains a public field, so code that builds one by struct literal must set it (none in the workspace). Host logging is a separate change. The comparison is evidence for the owner's review; no event-driven grant exists.
