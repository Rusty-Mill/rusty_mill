---
category: Fixed
changelog: The Mill Board backup script runs one at a time and never prunes from an emptied staging directory.
---
## 2026-11-27 - Mill Board backup: one run at a time

- **Fixed:** `deploy/backup.sh` takes an exclusive lock in the backup directory, so a hand-run `ship` cannot overlap the timer and rebuild the staging directory mid-run. Before pruning it also refuses to continue unless staging still holds exactly the verified pairs. Found by Codex review of the backup timer change; tests cover a refused second run and an emptied staging directory.
- **Known limitations:** The lock is per backup directory. Not installed on any host by this change.
