---
category: Added
changelog: **`rusty_mcp_proto`: `Schema` builder for a tool's `inputSchema`** (plan decision 2: builder first). `Schema::object().field(name, Kind, description, required).build()` covers strings, integers, numbers, booleans, string enums and string arrays; `property` takes a hand-built `Value` for anything richer.
---
## 2026-10-10 - rusty_mcp_proto: Schema builder for a tool's inputSchema

- **Added:** `rusty_mcp_proto::{Schema, Kind}`. `Schema::object().field(name, Kind, description, required).build()` builds the JSON Schema of a tool's arguments for strings, integers, numbers, booleans, string enums and string arrays; `property` takes a hand-built `Value` for anything richer. Redefining a name replaces it and its `required` flag; `required` is left out when empty. Lifted from the dropped parallel build (`c2329ddd`); the canonical proto crate had no schema module.
- **Tested:** 5 tests (exact output, empty schema, replacement, a verbatim property, and a `Tool` carrying a built schema decodes in `rmcp`); mutation check (`required` not cleared on redefinition) fails the replacement test.
- **Known limitation:** additive and unused so far; moving `rp-mcp` or `rusty_homelab_mcp` onto it is separate work.
