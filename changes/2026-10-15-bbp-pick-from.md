---
category: Changed
changelog: **BBP's event-driven pick rule is now one public function.** `wake::pick_from` is called by the engine, the shadow comparison and `bbp why`; nothing it decides, and no logged byte, changes.
---
## 2026-10-15 - BBP: `pick_from`, the pick rule in one place

- **Added:** `rusty_bbp::pick_from(&[(RoleId, WakeReason)], max_live_turns) -> Option<&(RoleId, WakeReason)>`: the first entry of a `wake_set` result that may take a turn (an observer only above one live turn), or `None`. It was written out three times (`pick_next`, `Shadow::comparison`, and a copy in `bbp why`); all three now call it. The fallback to the turn-based choice stays in `pick_next`.
- **Removed:** the host's copy of the rule in `bbp why`, and the test that guarded it against drifting from the engine.
- **Unchanged:** every grant and every logged byte. `pick_next` stays crate-private.
- **Added (tests):** `tests/pick_golden.rs` replays four event-driven runs (happy path, a question to a non-default role, two selectable entries, a quiet board) against logs recorded with the engine as it was before this change; a unit test checks `pick_next` against its previous form, kept as an oracle, over every state of those logs and the one-turn golden under both policies, with counts that fail it if no state exercises a pick or a fallback; `tests/wake.rs` covers `pick_from` itself. Before and after, all 201 deterministic logs of the `rusty_bbp` test suite (the 31 traces among them) were compared and are byte-identical; this comparison was a one-off run, not a committed test.
- **Known limitations:** none new.
