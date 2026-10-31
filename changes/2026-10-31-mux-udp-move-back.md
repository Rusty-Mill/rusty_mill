---
category: Added
changelog: rusty-mux moves a session back onto UDP once the path answers again after a fallback to SSH.
---
## 2026-10-31 - rusty_mux moves back to UDP after a fallback

- **Added:** moving back in `docs/projects/mux/UDP-TRANSPORT.md` (C13).
  - When a lost UDP path is brought back over SSH, the client keeps the association it gave up. Once that association hears the server again, the client probes it, takes every client over on it, and stops the SSH forward.
  - A client that stayed on SSH from the start, because UDP did not answer or the first move onto it failed, asks the server for a new association on the same schedule, and moves once one answers.
  - While the kept association is silent, the client also asks for a new one on the same schedule, in case the server let the old one go past `--udp-idle`.
  - A failed probe leaves the client on SSH and tries again 2 s later, doubling up to 30 s.
  - What is typed during a move, keys and focus changes alike, is held (up to 1024 actions) and taken in order once it is done, even on a path brought back after the move broke (with anything typed while it is down taken after them); keys typed into the focused block while its lease is asked for are sent once it is held, up to 64 KiB, even across a move or a lost path, but only to that block while it has the focus (otherwise they are dropped with a bell); a detach during a move waits for it to end, with nothing typed after it sent, so the clients it took over are detached on the path they end up on; a move that breaks after the control client moved tries the SSH forward in use before a new login; an SSH connection that ends during a move is a loss only if the move fails before any client moved.
- **Changed:** C13 is done in `docs/projects/mux/CAPABILITIES.md`: the four pieces the owner kept it open for have landed.
- **Known limitations:** with UDP blocked for good, the client asks again every 30 s, each time leaving the server an offer that lapses after 60 s.
