---
category: Changed
changelog: **BBP spec v0.5.0: roles are data, not protocol.** New `bbp-v0.5-implementation-spec.md` defines a generic Role (nine capability fields), roster injection at `task_opened`, one state-to-capability requirement table, and log schema 2; v0.4.1 is marked superseded.
---
## 2026-10-13 - BBP spec v0.5.0: roles are data, not protocol

- **Changed:** `docs/spec/bbp-v0.5-implementation-spec.md` replaces every named role in v0.4.1 with a capability check (R5, R6, R17, gating and by-state tables, T4, T9 to T11, resume guards, entry actions, visibility). A Role is an id plus `turn_order`, `post`, `write`, `read_artifacts`, `read_messages`, `verdict_at`, `final_decision`, `consultable`. The roster arrives at `task_opened`, is validated (ids, unique `turn_order`, every state servable including a finalizer), recorded in the log and immutable for the task.
- **Changed:** one requirement table derives each state's default turn holder and validates the roster. The consultation table becomes one rule; a human `ask` may address any assigned role, as the v0.4.1 engine already allowed. `final_decision`: at least one approve from such a role, and a `revise` from any of them blocks the merge gate; with one final role this is the single-Reviewer behavior.
- **Changed:** v0.4.1 is marked superseded. The four-role roster moves to Appendix C (non-normative, orch configuration); the Reviewer fresh-session requirement and Q3 move to orch.
- **Known limitations:** spec only. The engine still implements v0.4.1 until the follow-up engine change; log schema 2 does not replay schema 1 logs.
