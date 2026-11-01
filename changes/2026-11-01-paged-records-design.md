---
category: Added
changelog: rusty_multimodal_db gains a proposed design and ADR-0139 for keeping record bodies on disk in log-structured segments.
---
## 2026-11-01 - Paged records design (proposed)

- **Added:** `docs/design/STORAGE-PAGED-RECORDS-DESIGN.md`, which moves
  engine record bodies into append-only segment files with an in-RAM key
  directory and per-segment compaction, in phases M and 0–6.
- **Added:** ADR-0139 (proposed). It amends ADR-0138 decision 2: the
  `bincode` replacement ships first as a byte-identical first-party codec,
  with no format migration.
- **Known limitations:** documentation only; no code changes.
