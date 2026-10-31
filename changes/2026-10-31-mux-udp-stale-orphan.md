---
category: Changed
changelog: rusty-mux-server keeps a UDP client whose stream overflows its 8 MiB budget, for a takeover with a fresh screen, instead of dropping it.
---
## 2026-10-31 - rusty_mux stale orphans on UDP overflow

- **Changed:** overflow and catch-up in `docs/projects/mux/UDP-TRANSPORT.md` (C13).
  - A `TAKEOVER` client on a datagram stream whose output passes the 8 MiB budget no longer loses everything. Its stream ends and its queued output is discarded, but the client stays as a stale orphan for `--udp-idle`, with its attachment. Its lease is released after 30 s.
  - Its takeover keeps the client id and attachment, sends the block's screen again before any new output, and gives back the lease if it is still free.
  - Socket clients over budget are dropped as before.
- **Known limitations:** the client's 6 s outage display and moving back to UDP after a fallback to SSH come in their own PRs.
