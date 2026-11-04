---
category: Added
changelog: `rusty_orch watch` posts dependency, advisory and toolchain findings for shipped apps as claims on their maintenance tasks (#599).
---
## 2026-11-04 - Maintenance watcher: `rusty_orch watch`

- **Added:** `rusty_orch watch --factory DIR --bbp-dir DIR [--app NAME] [--dry-run]` reads `factory/apps/<app>.json` and runs three adapters in each app's repo: `cargo deny` advisories and yanked crates, `cargo update --dry-run` (one finding for the whole set), and the pinned toolchain against `factory/watched-versions.json`. It posts each finding as a low-stakes claim on the app's maintenance task, as the task's `watcher` role. The op ids are stable, so a finding already posted replays instead of repeating. It never opens a task and never merges (#599, D79, D80).
- **Known limitations:** the canary re-run on the released ref is not built yet. Each maintenance task needs a `watcher` role on its roster, opened by the human.
