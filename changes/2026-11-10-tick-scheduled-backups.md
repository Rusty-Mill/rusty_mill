---
category: Added
changelog: rusty_tick gains a nightly off-host backup timer and a restore drill that proves an archive opens and holds tasks.
---
## 2026-11-10 - Scheduled off-host backups for rusty_tick

- **Added:** `deploy/backup.sh` archives the stopped data directory with a sha256, keeps the newest N, and mirrors them to an rsync destination from `tick.env`. `rusty-tick-backup.service` and `.timer` run it nightly, stopping only rusty-tick for the snapshot; Mill Board answers 503 for those seconds. `deploy/restore-drill.sh` serves an archive from a temporary copy on a spare loopback port and counts each user's tasks. Both scripts are tested against the real binary (`tests/backup_scripts.rs`).
- **Known limitations:** Linux only (flock, systemd). Board settings and env files are not included. Shipping mirrors the local archives, so deleting them locally also removes them at the destination on the next run.
