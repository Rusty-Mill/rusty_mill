---
category: Added
changelog: Design note `docs/design/RLEVAL-EVALUATOR.md`, a gap analysis for RLEval's own evaluator and its first no-simulator milestone.
---
## 2026-10-11 - RLEval evaluator: gap analysis

- **Added:** `docs/design/RLEVAL-EVALUATOR.md`. Maps the evaluator brief (features, rank-tier profiles, gap analysis and suggestions, evaluation, data hygiene) onto what already exists (rank-assessment series, `RankNorms` and rank-relative scoring, skill detection, per-account habits) and defines milestone 1: a next-rank gap report from replay-level stats only, no simulator.
- **Known limitations:** design only. Rank label timing, party and smurf noise, and the number of matches needed for a non-provisional suggestion are open questions. The corpus is cross-sectional, so suggestions can only be described as descriptive, not as causes of rank change.
