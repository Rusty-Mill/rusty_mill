---
category: Added
changelog: Mill Board gains a daily SQLite backup timer with an integrity check, off-host mirroring and a written restore.
---
## 2026-11-27 - Mill Board SQLite backup timer

- **Added:** `deploy/backup.sh` makes a consistent online copy of `MILL_BOARD_DB` (the board keeps running), requires `integrity_check` ok on the copy, gzips it with a sha256, keeps the newest N complete pairs (the checksum is published last, so a lone gzip is never counted) and mirrors them to `<dest>/<host>/` over rsync pruning the destination only after it lists every pair just sent and never when local copies were wiped; `MILL_BOARD_BACKUP_REQUIRE_MOUNT` refuses to ship when the mount is absent. `mill-board-backup.service` and `.timer` run it daily; settings live in `backup.env`, not `board.env`, which the board refuses with `MILL_BOARD_BACKUP_*` lines. The deploy README documents install and restore.
- **Known limitations:** Linux only, needs python3. `settings/` is not in the SQLite file and is not backed up. Needs rsync and python3 on the host. Not installed on any host by this change.
