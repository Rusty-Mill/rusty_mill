---
category: Added
changelog: orch-lens value score and scheduling policies (event-driven baseline, value-driven) for #596.
---
## 2026-11-04 - orch-lens value score and scheduling policies

- **Added:** `orch_lens::value`, the #596 integer value score (uncertainty × stakes × reversibility × dependents × 100 / cost), and `orch_lens::policy` with the `Policy` trait, `EventDriven` (every job, schedule order) and `ValueDriven` (jobs of the top N claims by value). `round` takes a run policy and an optional shadow policy whose pick is reported in `Report::shadow`. `Card` carries `drop_at` and `built`.
- **Known limitations:** pure core only. Nothing persists shadow picks yet, and no command selects a policy until `rusty_orch lens run` lands. "The board asks" and fork-and-collapse come in later slices.
