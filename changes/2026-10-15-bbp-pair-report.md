---
category: Added
changelog: **BBP pair report.** `bbp pairs --manifest FILE` evaluates the 2b gate criteria over a run's pairs: the noise gate on the three null pairs, criteria 1 to 5 and 7, and the decision-table row that applies.
---
## 2026-10-15 - BBP pair report

- **Added:** `pairs::evaluate` (pure, over per-run facts) and `bbp pairs --manifest FILE`. The manifest lists the pairs (`null`, `separable`, `shared`) with, per run, the directory and task, the saved `bbp mod` output, and the grader's `rubric_ok`; per pair, `lost_rubric_items` from the grader. Facts read from the logs: closed, overshoot, `unused_conflicts`, `unused_bytes`; from `bbp mod`: the first `Planning` visit; from `bbp tokens`: tokens.
- **Implements the criteria note as written:** null ratios in 0.85 to 1.15 (a fail stops phase 2; fewer than three is not a pass); criterion 3 = median `spec_wall` ratio at most 0.75, at most one separable pair above 1.0, poll at most 5% of the control's `spec_wall` per pair, at least 4 measurable pairs, gate passed, else inconclusive; criterion 2 = 8 of 10 pairs and N=2 fails at most 1 more than N=1; criterion 7 = median token ratio at most 1.6, unknown tokens never a pass; failed pairs are out of every median. The decision table is applied first match.
- **Not decided here:** criterion 4 is reported as an upper bound (read the flagged versions by hand); criterion 6 is a build precondition and is not computed.
- **Thresholds are constants** at the top of the file, so a change the owner approves is one visible edit. This evaluates a run; it does not authorize one.
