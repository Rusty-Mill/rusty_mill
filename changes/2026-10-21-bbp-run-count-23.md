---
category: Changed
changelog: **BBP criteria: run count corrected to 23.** The Phase 2b note counted 26 runs; reusing each null task's first run as its phase 2 control gives 23, and the total spend cap follows (`23 x C`).
---
## 2026-10-21 - BBP criteria: run count 26 to 23

- **Changed:** `docs/bbp-concurrency-design.md` says 23 runs, not 26: 6 in phase 1, then 7 more controls and 10 treatments (the three null tasks' first runs are already their phase 2 controls, which the note already said). The same count is used in the phase 1 sentence and the two-tester variant's "never mixed into the 23".
- **Changed, spend:** the total cap line is now `23 x C`, one `C` per unique run, not `26 x C`. This lowers the cap by 3 per-run caps; the early-stop rule is unchanged. No margin is added: the owner can set one at run authorization.
- No pass mark, task count (10 pairs, 3 null pairs) or criterion changed; only the run count and the cap that derived from it. No code change.
- Docs only: no engine, schema or log path. The evidence run remains unqueued and needs separate owner authorization.
