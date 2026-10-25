---
category: Added
changelog: rusty_mux_proto request catalog, the schema the CLI and MCP front ends are generated from.
---
## 2026-10-25 - rusty_mux request catalog

- **Added:** `rusty_mux_proto::schema`. `CATALOG` lists every client request (all but the Hello handshake) as a `Method` with a kebab-case name, doc, typed `Param`s (session, window or block id, text, bytes, cell count) and what it returns (nothing, or the id it created). `Ty::parse` turns a front end's text argument into a `Value`; `Method::request` checks the arguments (unknown, missing, repeated, wrong type, zero size) and builds the wire `Message`. Tests pin that the catalog builds each request type 0x02 to 0x0b exactly once and that every built message round-trips.
- **Known limitations:** every parameter is required, and there is no optional or list type yet; they arrive with the first request that needs one. Server messages are not in the catalog.
