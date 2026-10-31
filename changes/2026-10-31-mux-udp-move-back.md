---
category: Added
changelog: rusty-mux moves a session back onto UDP once the path answers again after a fallback to SSH.
---
## 2026-10-31 - rusty_mux moves back to UDP after a fallback

- **Added:** moving back in `docs/projects/mux/UDP-TRANSPORT.md` (C13).
  - When a lost UDP path is brought back over SSH, the client keeps the association it gave up. Once that association hears the server again, the client probes it, takes every client over on it, and stops the SSH forward.
  - A failed probe leaves the client on SSH and tries again 2 s later, doubling up to 30 s.
- **Changed:** C13 is done in `docs/projects/mux/CAPABILITIES.md`: the four pieces the owner kept it open for have landed.
- **Known limitations:** a client that stayed on SSH from the start, because UDP never answered, does not try UDP again. Keys typed during the move, well under a second, may be lost, as during a reconnect.
