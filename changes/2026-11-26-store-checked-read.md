---
category: Fixed
changelog: orch-store: read-only table reads refuse a table whose mmap header `Store::open` would refuse (#1318 follow-up to #1903).
---
## 2026-11-26 - orch-store: read-only reads check the table header

- **Fixed:** `rusty_orch builder sessions` and the other read-only orch-store readers (`read_lens_reviews`, `read_meter`, `read_taints`) read records from a table's companion and insert log only, so a table whose mmap file had a truncated or corrupt header still read, while `Store::open` refused it (Codex on #1903). They now fail on that header too, and still make or write nothing.
- **Added:** `rusty_multimodal_db_engine`: `slot_file::check_header`, the header check of `SlotFile::open` on a read-only handle reading at most the header, and `GenericMmapStore::read_checked_records`, which runs it before `read_portable_records`. `read_portable_records`, `open` and every writer are unchanged.
- **Tests:** the engine checks a wrong magic, another version, a short and an empty file are refused with neither file changed; `builder sessions` fails on a corrupt table header with nothing repaired.
