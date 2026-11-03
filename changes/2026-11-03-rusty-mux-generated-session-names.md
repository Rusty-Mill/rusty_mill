---
category: Added
changelog: rusty_mux generated session names: a session created without a name gets one such as `quiet-harbor`, unique among the sessions.
---
## 2026-11-03 - rusty_mux generated session names (C50, part 1)

- **Added:** a session created with an empty name, by a client with the new `Features::SESSION_NAMES` (bit 29), gets an adjective-noun name that no other session has, such as `quiet-harbor`, or `quiet-harbor-2` once all 4096 pairs are taken. `rusty_mux_core::is_generated` tells such a name. A client without the bit still gets an empty name.
- **Changed:** `rusty-mux-ctl create-session` and the MCP tool take `--name` as optional; left out, ctl asks the server for a generated name and refuses against a server that cannot make one. The catalog's new `Presence::Defaulted` marks a parameter whose absence needs a feature, and `Method::all_needs` gives every feature a method can need.
- **Changed:** the attach client names the session it creates with a generated name instead of `main`, and still uses `main` with an older server.
- **Known limitations:** agents naming their session after their task (`agent setup --name-session`) is part 2 of C50.
