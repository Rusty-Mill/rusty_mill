---
category: Changed
changelog: **BBP criteria: killed-builder pairs.** The criteria note says a pair lost to a killed builder fails criterion 2 as well as leaving criterion 7, and tests pin the conditional bound.
---
## 2026-10-19 - BBP criteria: killed-builder pair semantics

- **Changed:** `docs/bbp-concurrency-design.md` states that a pair with a killed or interrupted turn counts as failed for criterion 2. Criterion 2 already bounds this (8 of 10 pairs succeed; N=2 failures exceed N=1's by at most 1), so with healthy N=1 controls the owner's 1-in-5 is that bound. Overlapping N=1 failures can permit additional N=2 failures under the unchanged rule.
- **Tests:** `with_healthy_controls_criterion_two_allows_one_killed_builder_pair` covers the conditional one-in-five bound; `overlapping_control_failure_allows_two_killed_builder_pairs` covers the existing `f2 <= f1 + 1` behavior. No behavior change.
- Docs and tests only: no engine, schema or log path. Criteria and run remain unapproved.
