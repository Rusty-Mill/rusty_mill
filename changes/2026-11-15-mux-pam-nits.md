---
category: Changed
changelog: PAM account checks end the PAM handle with the last call's status.
---
## 2026-11-15 - rusty_mux PAM review nits (#1460)

- `pam_end` now gets the status of the last PAM call (the `pam_acct_mgmt` result) instead of `pam_start`'s, so modules see the real status when they clean up. RISKS R17 notes that up to 16 checks run at once and a non-reentrant third-party module may not allow it.
