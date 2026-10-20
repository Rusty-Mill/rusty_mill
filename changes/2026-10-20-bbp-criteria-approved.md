---
category: Changed
changelog: **BBP Phase 2b criteria approved.** The owner approved the criteria and prices on 2026-10-20; `docs/bbp/2b-criteria.json` no longer carries the `status` proposal marker. No evidence run is queued.
---
## 2026-10-20 - BBP Phase 2b criteria approved

- **Changed:** the owner approved `docs/bbp-concurrency-design.md` (criteria 1 to 7 and the decision table) and `docs/bbp/2b-criteria.json` (prices checked 2026-10-19). The `status` key is deleted, as its text said to do on approval. The design note's status line and price paragraph now say approved and confirmed.
- **Decided with it:** killed-builder pairs fail their pair and count against criterion 2 (#802); the grader's residual text-similarity limit is accepted as stated in the note.
- Unchanged: Haiku unpriced and refused, subagents forbidden, `min_commit` pinned. Approval fixes the pass marks only: nothing is queued, and the evidence run needs its own go.
- Docs and data only: no engine, schema or log path.
