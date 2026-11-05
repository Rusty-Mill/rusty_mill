---
category: Added
changelog: `rusty_orch lens record` prints each lens's track record on one task as one JSON line, D93's calibration report (#599, D111).
---
## 2026-11-05 - `lens record` calibration report

- **Added:** `rusty_orch lens record --bbp <bbp> --dir <bbp dir> --task <task> --state <orch state>` (#599, D111). It prints one JSON line per lens, in name order: `points`, `assigned`, `scored`, `pending`, `pivotal` and `pivotal_with_truth`, read from the blind reviews `lens run` saved and each claim's `bbp why` history. It posts and saves nothing, and refuses a `--state` that doesn't exist rather than making one.
- **Known limitations:** the numbers carry the record's interim limits until BBP's history carries more (D112): pivotal jobs are counted conservatively and golden truth isn't read on this path yet. The store is locked while its rows are read, so it fails with "in use" while a `lens run` holds the same `--state`.
