---
category: Changed
changelog: **BBP criteria: killed-builder pairs.** The criteria note says a pair lost to a killed builder fails criterion 2 as well as leaving criterion 7, and a test pins the bound (at most one such pair in an N=2 arm).
---
## 2026-10-19 - BBP criteria: killed-builder pair semantics

- **Changed:** `docs/bbp-concurrency-design.md` states that a pair with a killed or interrupted turn counts as failed for criterion 2. Criterion 2 already bounds this (8 of 10 pairs succeed; N=2 failures exceed N=1's by at most 1), so no new threshold: the owner's 1-in-5 is that bound.
- **Tests:** `killed_builders_fail_criterion_two_once_more_than_one_pair_in_five_is_lost` (0 and 1 lost pairs hold, 2 fail with the "Inconclusive" row). No behavior change.
- Docs and tests only: no engine, schema or log path. Criteria and run remain unapproved.
