---
category: Added
changelog: rusty_mux: systemd user units with socket activation for rusty-mux-server, and a plain "no server is running" from rusty-mux-ctl (C69 slice c).
---
## 2026-11-07 - rusty_mux systemd units and socket activation

- **Added:** `rusty-mux.socket` and `rusty-mux.service` user units in `crates/apps/rusty_mux/crates/server/dist/systemd/`, installed by hand. The server takes its listening socket from systemd when started by the socket unit, binds nothing of its own and leaves the socket's name to systemd.
- **Changed:** `rusty-mux-ctl` says "no server is running: nothing at PATH" when nothing is at the socket, instead of the raw connect error (`WSAENETDOWN` on Windows).
- **Known limitations:** no installer command and no Windows service; utmp entries are not written.
