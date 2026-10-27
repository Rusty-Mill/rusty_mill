---
category: Added
changelog: rusty_remind_me: scripts/install_cloud_capture.py installs the cloud session capture hook into any repository (ADR-0029).
---
## 2026-10-27 - remind-me: install cloud capture in other repositories

- **Added:** `crates/apps/rusty_remind_me/scripts/install_cloud_capture.py TARGET` copies `.claude/hooks/remind-me-capture.py` into another repository and registers it on `Stop` and `PreCompact`, keeping that repository's other settings and hooks. Re-running it updates the copy without adding a second registration. `--check` exits 1 when the copy or registration is stale. A settings file it cannot safely edit is refused untouched.
- **Known limitations:** each repository commits its own copy, so a hook change reaches it only when the installer is re-run there.
