---
category: Changed
changelog: Merge-queue CI runs restore Rust caches but no longer save them, since no other run can read a queue ref's cache.
---
## 2026-10-21 - Merge-queue runs skip cache saves

- **Changed:** every `Swatinem/rust-cache` step in `ci.yml` and the
  `setup-build-env` action sets `save-if` to skip `merge_group` runs. A queue
  run's cache is scoped to its temporary `gh-readonly-queue/...` ref, so no
  other run can restore it; the Windows upload alone held #835's merge for
  over 15 minutes. Queue runs still restore `main`'s caches.
