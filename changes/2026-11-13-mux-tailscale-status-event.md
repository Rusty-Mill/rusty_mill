---
category: Added
changelog: rusty_mux servers on a tailnet send a tailscale-status-changed event, behind a new feature bit.
---
## 2026-11-13 - rusty_mux tailscale-status-changed event (C27 slice 4)

- **Added:** event kind 34 `tailscale-status-changed` and `Features::TAILSCALE_STATUS` (bit 45, offered on Linux). A shared server with `--tailnet` asks `tailscaled` every 5 s and sends the node's state (`running`, `needs-login`, `stopped`, `other`) and name when either changes. It is sent to tailnet connections only (`ts:` and `ts-node:` principals) that negotiated bit 45, live and on replay. Two failed polls in a row read as `other`; one failure is logged once and recovery once. Older clients do not see it. `rusty-mux-ctl events` and `watch` show it (card #276, rusty_term D265).
