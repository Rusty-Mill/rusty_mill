---
category: Added
changelog: Login mode can ask the site's PAM policy whether an account may be served (--pam).
---
## 2026-11-14 - rusty_mux PAM account checks (#1460)

- **Login mode:** `rusty-mux-server --login MAP --pam[=SERVICE]` (off by default) asks PAM's `pam_acct_mgmt` for the mapped account before its server starts (`/etc/pam.d/rusty-mux` by default), so expired, locked and time-limited accounts are refused. Account checks only: no passwords and no `pam_open_session`. libpam is loaded with `dlopen` at start (no link, no `-dev` package, no new crate); the front door refuses to start if it, the service file or a probe `pam_start` fails, and any error, timeout or expired-password answer refuses a connection. A refusal is a closed socket and a stderr line. New `platform_linux::Pam`. No wire change. Pack rows D266, D267, D273.
