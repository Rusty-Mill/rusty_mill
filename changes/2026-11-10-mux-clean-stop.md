---
category: Added
changelog: rusty_mux stops cleanly on request (`rusty-mux-ctl server stop`) and on SIGTERM/SIGINT, saving sessions for an upgrade.
---
## 2026-11-10 - Mux clean stop (C36b)

- **Added:** client message `Shutdown` (type 0x24) behind `Features::SHUTDOWN` (bit 38). Only a client of the server's own account may send it; the server answers `Done`, then saves its store, hangs up owned blocks, releases its socket name and closes connections last. Not a catalog method.
- **Added:** `rusty-mux-ctl server stop`: waits until the server has stopped; exits 0, 4 when no server answers, 1 when refused or the server predates `SHUTDOWN`, 124 after 15 s.
- **Changed:** on Linux, SIGTERM and SIGINT are clean stops, as Ctrl+C and Ctrl+Break are on Windows. Before, a signal killed the server without saving and left its socket name.
- **Docs:** C36-UPGRADE (b), C69-LIFECYCLE, CAPABILITIES C36, a Proposed mux DECISIONS row; pack D160-D164 and Q36 (D161, D164 and Q36 record arc 1's go and build defaults for the next PR).
- **Known limitations:** under systemd socket activation the socket stays, so the next connection starts a new server. Windows has no SIGTERM; its stops are Ctrl+C, Ctrl+Break and the request.
