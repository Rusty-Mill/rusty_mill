---
category: Added
changelog: rusty-mux-ctl ssh check|diagnose|setup checks a host for rusty-mux --ssh without connecting and prints a Host block for ~/.ssh/config (C48).
---
## 2026-11-10 - rusty-mux-ctl ssh check, diagnose and setup

- **Added:** `rusty-mux-ctl ssh check DEST` reports what would stop `rusty-mux --ssh DEST` from working, from what `ssh -G` resolves, without connecting. It flags OpenSSH older than 8.7, `ClearAllForwardings`, and a host key no known-hosts file has, and warns about missing keys, proxies and Windows. `ssh diagnose DEST` also prints every value it used and a fix for each finding. `ssh setup DEST` prints a marked `Host` block to add, or says the config already has one.
- **Known limitations:** nothing is written: `setup` only prints. Includes with wildcards are not followed when looking for an existing host.
