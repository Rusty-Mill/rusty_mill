---
category: Added
changelog: `rusty_orch lens record` prints each lens's track record on one task as one JSON line, D93's calibration report (#599, D111).
---
## 2026-11-05 - `lens record` calibration report

- **Added:** `rusty_orch lens record --bbp <bbp> --dir <bbp dir> --task <task> --state <orch state>` (#599, D111). It prints one JSON line per lens, in name order: `points`, `assigned`, `scored`, `pending`, `pivotal` and `pivotal_with_truth`, read from the blind reviews `lens run` saved and each claim's `bbp why` history. It posts, saves and writes nothing: `Store::read_lens_reviews` (new) takes the store's existing lock without creating it (`DirLock::acquire_existing`, new in `rusty_multimodal_db_engine`) and reads the lens record table's blob and insert log as `Store::open` would recover them, leaving every file byte for byte as it was. It refuses a `--state` without a lock or table rather than making one.
- **Known limitations:** the numbers carry the record's interim limits until BBP's history carries more (D112): pivotal jobs are counted conservatively and golden truth isn't read on this path yet. The store is locked while its rows are read, so it fails with "in use" while a `lens run` holds the same `--state`.
