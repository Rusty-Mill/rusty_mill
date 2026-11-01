---
category: Changed
changelog: rusty_mux records the UDP transport as built, with a manual check over a real sshd; C13 stays in progress until its last pieces land.
---
## 2026-10-31 - rusty_mux UDP transport built, manual check

- **Changed:** build step 4 of `docs/projects/mux/UDP-TRANSPORT.md` (Phase 3 step 4, C13).
  - CAPABILITIES adds the C13 evidence. C13 stays in progress until the four unbuilt pieces below land (owner, 2026-10-31).
  - The design doc's status is now "built".
  - `UDP-MANUAL-CHECK-2026-10-31.md` records `rusty-mux --ssh` through a real local sshd. The client moves onto UDP and stops SSH when the server offers it. It stays on SSH with `RUSTY_MUX_UDP=off`, with a refused or silent port, and with a server without `--udp`.
- **Known limitations:** the roam is tested on loopback only, not under a real sshd. Still unbuilt: the 6 s outage display, the lease release after 30 s of silence, the stale orphan on overflow, and moving back to UDP after a fallback to SSH.
