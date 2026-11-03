---
category: Changed
changelog: The BBP event-based plan drops the live four-editor model check; step 4 is measured with scripted editors only.
---
## 2026-11-01 - BBP step 4 without a live model run

- **Changed:** `docs/bbp-event-based-plan.md` drops the live N = 4 model check from step 4, by the owner's decision of 2026-11-01: BBP runs use scripted editors only and never a paid API key. The step 4 decision stands on the scripted harness (`rusty_bbp/tests/coedit_load.rs`).
- **Known limitations:** the scripted edit rates are taken from the 2b planner traces and are not confirmed against live co-editing agents.
