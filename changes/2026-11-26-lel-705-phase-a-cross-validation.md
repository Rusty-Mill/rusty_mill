---
category: Added
changelog: Let Em Loose #705 Phase A: rusty_erosion cross-validated against SlopCodeBench's scorer on 196 reference checkpoints.
---
## 2026-11-26 - Let Em Loose: #705 Phase A cross-validation

- **Added:** `crates/tools/rusty_erosion/scb/` (`score.sh`, `summarize.py` and its tests), `examples/measure_tree.rs`, and the report `docs/projects/let-em-loose/705-slopcodebench-phase-a.md` with its committed rows. Concentration vs SCB erosion gives Spearman 0.95 with 84% direction agreement, and duplication vs clone share gives 0.64, so both pre-registered rules (D201) pass.
- **Known limitations:** the scores come from Python reference solutions only, scored with scb-check 0.1.3; the Rust adapter isn't tested directly.
