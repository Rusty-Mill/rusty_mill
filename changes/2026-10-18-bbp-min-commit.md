---
category: Changed
changelog: **BBP criteria.** `docs/bbp/2b-criteria.json` pins `min_commit` to `735f1b25`, the merge commit that contains both the verdict-binding fix (#727) and the run tooling (#763).
---
## 2026-10-18 - BBP criteria: `min_commit`

- **Changed:** `min_commit` was `null` until both #727 and #763 were merged. It is now `735f1b25e8479424181733756307e0bba8eef200`, the merge of #763 (which includes #727). `bbp pairs` refuses a checkout whose HEAD does not contain it. `prices_as_of` and `status` are unchanged: the owner updates them. The criteria and the run remain unapproved.
- **Tests:** `the_committed_criteria_pin_min_commit_to_a_full_commit_id` (red before the change).
- Host and docs only.
