---
category: Added
changelog: Design note for PAM account checks in rusty-mux-server login mode.
---
## 2026-11-14 - rusty_mux PAM design note (#1460)

- **Docs:** `docs/projects/mux/PAM.md` proposes `--pam` account checks (`pam_acct_mgmt` only, libpam loaded with `dlopen`, fail closed) for login mode. Design only; nothing is built. Pack rows D266, D267, Q65.
