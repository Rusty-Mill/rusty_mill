---
category: Added
changelog: rusty_tick ships a systemd unit and install guide, and Mill Board's unit starts after it on the same host.
---
## 2026-10-27 - rusty_tick as a service beside Mill Board

- **Added:** `crates/apps/rusty_tick/deploy/` with `rusty-tick.service`, `tick.env.example` and an install guide: loopback on 8788 so Mill Board keeps 8787, the web UI served from a copy of `web/dist`, and how to move an existing data directory. `mill-board.service` is ordered after `rusty-tick.service`, without requiring it.
- **Known limitations:** rusty_tick has no graceful shutdown, so `systemctl stop` ends it at once; back up its data with the service stopped.
