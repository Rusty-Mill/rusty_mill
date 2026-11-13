---
category: Added
changelog: rusty-mux-server can ask tailscaled who a tailnet address is and name a principal for it (first slice of C27).
---
## 2026-11-13 - rusty_mux tailnet identity, slice 1

- **Added:** `rusty_mux_server::tailnet::Whois` (Linux), the first slice of C27 (card #276, Arc 1): it asks `tailscaled`'s LocalAPI `whois` and names a principal, `ts:LOGIN` for a user's node and `ts-node:NAME` for a tagged one. An unknown peer is refused. Nothing listens on the tailnet yet.
- **Added:** `ts_types::tailcfg::WhoIsResponse` and `Node::tags`.
- **Changed:** `rusty_mux_server` depends on `rusty_http`, `serde_json` and `ts-types`, all already in the workspace.
