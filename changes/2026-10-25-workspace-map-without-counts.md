---
category: Changed
changelog: docs/WORKSPACE-MAP.md drops the dependents-count column; generate_workspace_map.py --counts prints the counts on demand.
---
## 2026-10-25 - Workspace map without dependents counts

- **Changed:** `docs/WORKSPACE-MAP.md` no longer has a dependents-count column. The count changed on nearly every dependency edit (30 of the 33 rows that changed from 2026-10-10 to 10-25 changed only that cell), so unrelated PRs rewrote the same rows, git line-merged them into duplicates, and `dependency policy` failed on PRs, queue entries and once on `main` (#1034). A row now changes only when its crate is added, renamed or redescribed.
- **Added:** `generate_workspace_map.py METADATA --counts` prints `crate<TAB>dependents`, most depended-on first.
- **Known limitations:** two PRs that both add a crate in the same family can still need a regeneration after merging main; that is rare by comparison. ADR-0003 is amended to match.
