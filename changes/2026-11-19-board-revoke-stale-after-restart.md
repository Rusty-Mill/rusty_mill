---
category: Fixed
changelog: Mill Board's stale-agent revoke now finds idle agents right after a board restart instead of finding nothing for 24 hours.
---
## 2026-11-19 - Mill Board revoke stale works after a restart

- **Fixed:** after a restart, an agent loaded from the registry and not used since counted as last used at the restart, so `settings_registered_agent_revoke_stale` and the Settings button found nothing until the chosen age had passed since the board started (a day at the default, after every deploy). Its last use now counts as the saved last use plus the hour the file may lag, never later than the load.
- **Known limitations:** a use whose write to the registry file failed (logged as a warning) is not on disk, so after a restart that agent can look idle and be revoked; it must register again (owner decision, 2026-11-19).
