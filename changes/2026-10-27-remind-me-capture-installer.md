---
category: Added
changelog: rusty_remind_me: scripts/install_cloud_capture.py installs the cloud session capture hook at user level, so an environment's setup script captures every session, including multi-repository ones (ADR-0029).
---
## 2026-10-27 - remind-me: capture every session of a cloud environment

- **Fixed:** a cloud session that checks out several repositories starts in `/home/user` and loads no repository's `.claude/settings.json`, so the capture hook never ran there.
- **Added:** `crates/apps/rusty_remind_me/scripts/install_cloud_capture.py [--home DIR] [--source HOOK]` copies the hook into `~/.claude/hooks/` and registers it on `Stop` and `PreCompact` in `~/.claude/settings.json`, keeping other settings and hooks. A setup-script snippet in the README fetches both files at a pinned commit and runs it. Re-running repairs a differing or duplicate registration without adding another. `--check` exits 1 on drift. Symlinked or malformed paths and settings are refused untouched. Writes are staged and replaced atomically, with rollback.
- **Known limitations:** the snippet pins a commit, so a hook change reaches an environment only when its `REF` is bumped.
