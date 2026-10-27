---
category: Added
changelog: rusty_mux labels and client management: rename sessions, windows, blocks and clients, see renames as events, list connected clients and disconnect one.
---
## 2026-10-27 - rusty_mux labels and client management (C33)

- **Added:** names on blocks and clients, alongside sessions and windows, and the methods `rename-session`, `rename-window`, `rename-block` and `rename-client` (`rusty-mux-ctl rename-block --block 1 --name build`, and MCP tools). They travel as one `Rename` message; renaming to the current name changes nothing.
- **Added:** `session-renamed`, `window-renamed`, `block-renamed` and `client-renamed` events (kinds 21 to 24) with the entity's id and new `name`, for `events`, `watch` rules and `wait --event`.
- **Added:** `list` reports each connected client (`client ID name="…" attached=BLOCK`), ctl's own connection included; `disconnect-client` ends another client's connection, detaching it and freeing any input lease it holds. ctl targets resolve blocks and clients by name, position or id suffix.
- **Changed:** the protocol gains `Features::LABELS` (bit 14), which gates renames, renamed events, client entries and disconnect; block entries carry a `name` field that older clients ignore.
- **Security:** any client on the socket may disconnect any other (not itself) and so take its lease; this matches the current single-user trust model and needs restricting before shared or login servers (Phase 3).
