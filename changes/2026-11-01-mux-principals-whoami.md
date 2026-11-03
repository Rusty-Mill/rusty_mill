---
category: Added
changelog: rusty_mux shows who each client is and how it is connected (`whoami`, principals in `list`, `client-connection-changed`).
---
## 2026-11-01 - rusty_mux: principals, whoami and connection changes

- **Added:** every client in `rusty-mux-ctl list` (and the MCP `list` tool) now shows the principal the server verified (`uid:N` and the account name on Unix, `sid:S-…` on Windows) and its transport, `socket` or `udp`. Phase 3 identity step D8 (C25).
- **Added:** `rusty-mux-ctl whoami` and the MCP `whoami` tool show the caller's own client.
- **Added:** a `client-connection-changed` event, with the new transport, when a reconnect or a move to or from UDP takes a client over. Rules and `wait --event` can name it.
- **Changed:** protocol feature `IDENTITY` (bit 22). Against a server without it, ctl and MCP refuse `whoami` before sending anything, and older clients never receive the new event. Client entries carry the new fields to every client, and older decoders skip them.
- **Known limitations:** a personal server serves only its owner, so every client has the same principal until login mode (C26). Windows shows the SID without an account name.
