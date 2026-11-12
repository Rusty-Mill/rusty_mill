---
category: Added
changelog: rusty-mux on Windows can attach to a server on another machine with `--ssh HOST`, through `rusty-mux-ctl bridge` over SSH.
---
## 2026-11-12 - rusty_mux: `rusty-mux --ssh HOST` on Windows (C79)

- **Added:** on Windows, `rusty-mux --ssh HOST <socket> [block]` binds a private local socket (owner-only DACL, and each connection's process checked against this user) and joins each connection to its own `ssh HOST rusty-mux-ctl --socket <socket> bridge`. Windows' OpenSSH cannot forward a local Unix socket, which the Unix route uses (C79 spike). The first login may ask for a password or a host key; every later one runs in BatchMode, so a key or an agent is needed. `RUSTY_MUX_REMOTE_CTL` names the remote ctl as shell text when a non-login shell's `PATH` does not reach it. A lost path logs in again and takes the clients over, as on Unix.
- **Changed:** `rusty-mux-ctl ssh check` on Windows no longer says `--ssh` is Unix only, and skips the forward and UDP findings, which do not apply to the bridge.
- **Known limitations:** UDP stays Unix only: a Windows client stays on SSH. Every connection is a separate SSH login.
