---
category: Changed
changelog: The rusty_tailscale library crates the embedded mesh node needs moved from crates/apps/rusty_tailscale to crates/libs/net.
---
## 2026-11-15 - rusty_tailscale node crates move to libs/net

- **Changed:** `ts-net`, `ts-engine`, `ts-key`, `ts-control`, `ts-derp`, `ts-wg`, `ts-tun`, `ts-magicsock`, `ts-disco`, `ts-filter` and `ts-stun` moved from `crates/apps/rusty_tailscale/crates/` to `crates/libs/net/`, and their layer is now `libs`. Same names, same code; the only edit is `ts-control`'s relative path to `rusty_h2`. `ts-daemon`, `ts-cli`, `ts-localapi` and `xtask` stay in `apps/`.
- **Why:** the embedded mesh node (MESH.md stage 1) lets `rusty_mux_server` depend on `ts-net`, and ADR-0003 forbids an app depending on another app. This is the move gate of rusty_term D233 and Q60. Nano chose to do the move first, as its own step (2026-11-15). ADR-0003's family table and appendix, the READMEs and the generated workspace map are updated.
