---
category: Added
changelog: `rusty_orch watch` posts dependency, advisory and toolchain findings for shipped apps as claims on their maintenance tasks (#599).
---
## 2026-11-04 - Maintenance watcher: `rusty_orch watch`

- **Added:** `rusty_orch watch --factory DIR --bbp-dir DIR --state DIR [--app NAME] [--dry-run]` reads `factory/apps/<app>.json` and runs three adapters in each app's repo: `cargo deny` advisories and yanked crates, `cargo update --dry-run` (one finding for the whole set), and the pinned toolchain against `factory/watched-versions.json`. It posts each finding as a low-stakes claim on the app's maintenance task, as the task's `watcher` role. The orch-store `watch_finding` table (D58, D83) remembers each finding's claim. A finding already posted isn't posted again, even when its details change, and the details are shown in the report only. A newer update set or watched Rust version posts a claim that supersedes the open one. Op ids and payloads are stable, so a post whose table write was lost replays the same claim. It never opens a task and never merges (#599, D79, D80).
- **Known limitations:** the canary re-run on the released ref is not built yet. Each maintenance task needs a `watcher` role on its roster, opened by the human.
