---
category: Changed
changelog: rusty_mux C36 (upgrade without losing sessions) is closed at the Rex level, with a documented check.
---
## 2026-11-16 - rusty_mux C36 close-out

- **Changed:** C36 reads done: a running server keeps its version and sessions, `server stop` then the next start restores them. Added the close-out check to C36-UPGRADE.md.
- **Known limitations:** The check was run on Linux only; Windows is not yet run. Live re-exec (#1011) stays a spike.
