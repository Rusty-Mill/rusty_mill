---
category: Fixed
changelog: The daily CI sweep deletes the rust caches it supersedes, so the 10 GB cache budget stops evicting live ones.
---
## 2026-10-26 - Prune superseded CI caches after the daily sweep

- **Fixed:** rust-cache keys end in a Cargo.lock/manifest hash, so each sweep of a changed `main` saved new caches beside the old ones. Storage reached 9.81 of 10 GB, and least-recently-used eviction took the live Linux clippy cache (saved 2026-10-25 08:53, missing for every run afterwards). A new `prune-caches` job runs after the cache-saving jobs on `main` and deletes every `v0-rust-*` entry older than the newest of its family (`.github/scripts/stale_caches.py`), about 3 GB a day. It is skipped on pull requests and in the merge queue.
- **Known limitations:** families that stop being saved (renamed jobs, a runner image's new environment hash) are left to GitHub's eviction. The job has `actions: write`, which it needs to delete caches.
