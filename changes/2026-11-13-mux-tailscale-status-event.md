---
category: Added
changelog: rusty_mux servers on a tailnet send a tailscale-status-changed event, behind a new feature bit.
---
## 2026-11-13 - rusty_mux tailscale-status-changed event (C27 slice 4)

- **Added:** event kind 34 `tailscale-status-changed` and `Features::TAILSCALE_STATUS` (bit 45, offered on Linux). A shared server with `--tailnet` asks `tailscaled` every 5 s and sends the node's state (`running`, `needs-login`, `stopped`, `other`) and name when either changes. Older clients do not see it. `rusty-mux-ctl events` and `watch` show it (card #276, rusty_term D265).
