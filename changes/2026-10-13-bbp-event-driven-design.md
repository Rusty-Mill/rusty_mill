---
category: Added
changelog: BBP event-driven participation design note (docs only): trigger conditions, merge versus delivery, scheduling seam, anti-ping-pong controls, and a recommendation.
---
## 2026-10-13 - BBP event-driven participation design

- **Added:** `docs/bbp-event-driven-design.md`. Recommends a pure `wake_set` over the nine Role fields, a narrow `pick_next` scheduling seam beside the turn-based default, one live turn kept, delivery owned by the host/orch, and rejects CRDT merge and remind-me as the wake path.
- **Known limitations:** Design only, nothing implemented. Remind-me webhook outbound capability and `rusty_multimodal_db` transaction semantics were not verified; DeltaDB delivery is not public. Depends on the v0.5.0 Role refactor.
