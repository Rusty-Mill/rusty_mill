---
category: Changed
changelog: bbp human extend refuses on an evidence run, and bbp pairs refuses a run whose log holds a budget extension in any field.
---
## 2026-11-01 - bbp: no budget extensions in evidence runs

- **Changed:** `bbp human extend` (and the fenced form Orch uses) refuses, for every budget field, on a task that carries an evidence marker, and appends nothing. A marker that cannot be read refuses too; only a specific not-found means the task is not an evidence run. Owner decision, 2026-11-01: an evidence run runs under the budget it was opened with.
- **Changed:** `bbp pairs` refused only a `reads_per_turn` extension (#1327). It now refuses a run whose log holds an extension of any field, naming the field, the opened value and the new limit. This also closes the gap where an extended `messages` or `bytes` total hid an earlier overshoot, which is checked against the folded budget.
- **Known limitations:** an evidence run that exhausts a budget can no longer be resumed; it stays escalated and counts under the 2b rules for such a run. Extensions on tasks with no evidence marker are unchanged.
