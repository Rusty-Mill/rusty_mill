---
category: Added
changelog: rusty-mux-server releases a UDP client's input lease after 30 s of silence, and gives it back when the client is heard again if nobody took it.
---
## 2026-10-31 - rusty_mux UDP lease release after silence

- **Added:** the lease part of lifetime B in `docs/projects/mux/UDP-TRANSPORT.md` (C13).
  - An association silent for 30 s releases the input lease each of its clients holds, once per silence, so other clients can type while a laptop sleeps. The association and its clients stay, up to `--udp-idle`.
  - When the association is heard again, each client gets its lease back if it is still attached to that block and the lease is still free. A lease another client took meanwhile stays with them.
  - A client taken over by another connection is no longer the association's, so its lease is left alone.
- **Known limitations:** the stale orphan on overflow, the client's 6 s outage display and moving back to UDP after a fallback to SSH come in their own PRs.
