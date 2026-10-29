---
category: Added
changelog: rusty-mux --ssh reconnects by itself after the path to the server goes silent, keeping the screen and the input lease.
---
## 2026-10-29 - rusty_mux: `rusty-mux --ssh` reconnects

- **Added:** with `--ssh`, the client asks the server for a sign of life every 5 s. After 15 s without one, or when a connection drops, it logs in again with `BatchMode=yes` beside the old `ssh` and takes its clients over, so the block's screen comes back and typing carries on. Only then is the old `ssh` stopped. A failed login is retried with backoff up to 30 s; Ctrl-] d still detaches. Phase 3 step 2c; DECISIONS 2026-10-29.
- **Known limitations:** a reconnect needs key or agent authentication, since the client owns the terminal. If the server let the clients go first (the old `ssh` died and sshd closed its channels), the client stops instead of starting over. Keys typed while the path is down are dropped, and nothing on screen says the client is reconnecting.
