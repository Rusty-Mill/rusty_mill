---
category: Added
changelog: rusty-mux moves a session back onto UDP once the path answers again after a fallback to SSH.
---
## 2026-10-31 - rusty_mux moves back to UDP after a fallback

- **Added:** moving back in `docs/projects/mux/UDP-TRANSPORT.md` (C13).
  - When a lost UDP path is brought back over SSH, the client keeps the association it gave up. Once that association hears the server again, the client probes it, takes every client over on it, and stops the SSH forward.
  - A client that stayed on SSH from the start, because UDP did not answer, asks the server for a new association on the same schedule, and moves once one answers.
  - A failed probe leaves the client on SSH and tries again 2 s later, doubling up to 30 s.
- **Changed:** C13 is done in `docs/projects/mux/CAPABILITIES.md`: the four pieces the owner kept it open for have landed.
- **Known limitations:** with UDP blocked for good, the client asks again every 30 s, each time leaving the server an offer that lapses after 60 s. Keys typed during the move, well under a second, may be lost, as during a reconnect.
