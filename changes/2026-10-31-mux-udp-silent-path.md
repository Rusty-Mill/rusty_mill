---
category: Changed
changelog: rusty-mux notices a silent UDP path after 6 s, not 15 s, and brings the session back over SSH.
---
## 2026-10-31 - rusty_mux loses a silent UDP path in 6 s

- **Changed:** liveness in `docs/projects/mux/UDP-TRANSPORT.md` (C13).
  - On UDP, the client's heartbeat also reads when the relay last heard the server. The server sends a keepalive after 3 s of silence, so 6 s with nothing heard counts as two missed keepalives, and the path is lost then.
  - A lost path is shown and brought back as before (#1243): a 2 s probe of the association, then a second login over SSH.
  - An SSH forward still waits the 15 s missed sign of life.
- **Known limitations:** after a fallback to SSH the client stays there; moving back to UDP comes in its own PR.
