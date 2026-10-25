---
category: Changed
changelog: CI restores main's Rust caches in scoped runs, lints PRs on Windows, and files an issue when the nightly sweep fails.
---
## 2026-10-24 - CI: warm caches, Windows clippy on PRs, nightly alert

- **Changed:** clippy and test cache keys no longer include the planner component, so scoped PR and merge-queue runs restore the daily sweep's cache instead of building cold.
- **Changed:** only `main` saves Rust caches; PR saves were unrestorable by other runs and evicted `main`'s from the 10 GB quota.
- **Changed:** pull requests run clippy on Windows as well as Linux; Windows clippy was the most common reason a queue entry was ejected.
- **Added:** `nightly-alert.yml` opens an issue when the daily sweep fails, comments on repeat failures and closes it when the sweep is green.
- **Known limitations:** larger cache restores (several GB on Windows) and some dependency rebuilds from feature unification in scoped runs; repeat pushes to a PR no longer reuse that PR's own cache. Windows tests still run only in the queue. The alert workflow takes effect once merged to `main`.
