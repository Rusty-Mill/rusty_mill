---
category: Added
changelog: rusty-mux-ctl --host now reaches a saved host's ssh addresses through the same OpenSSH forward rusty-mux --ssh uses (C35).
---
## 2026-11-10 - rusty-mux-ctl reaches ssh addresses

- **Added:** `rusty-mux-ctl --host NAME` reaches an `ssh` address through an OpenSSH socket forward that logs in without asking (`BatchMode`), waiting up to 10 s. A login failure falls through to the next address, like an unreachable socket.
- **Changed:** the forward moved from `rusty_mux_tui` to `rusty_mux_client::ssh`, which the terminal client and `rusty-mux-ctl` now share. `ssh check` takes its options from there. `rusty_mux_ctl` and `rusty_mux_mcp` now declare rust-version 1.98.1.
- **Known limitations:** ssh addresses are skipped on Windows. ssh's own error output goes to ctl's stderr. Cancelling does not interrupt the up-to-10 s wait for a forward.
