---
category: Changed
changelog: remind-me's MCP tools are grouped and merged from 75 to 43, with old names kept as aliases until 0.6.0 (ADR-0027).
---
## 2026-10-26 - remind-me tool groups and merges

- **Changed:** every remind-me MCP tool now belongs to one of six groups (`core`, `pipeline`, `wiki`, `import`, `admin`, `extras`), and `REMIND_ME_TOOL_PROFILE` takes a comma list of them, such as `core,wiki`. `core` is ten tools: search, get, add, update, delete, auto_capture, resolve, entity, digest and server_status. `full` (still the default) and `standard` keep working.
- **Changed:** look-alike tools merged into one tool that routes on an argument, so the full surface is 43 tools instead of 75: `search` also lists and looks up references, `get` also reads captures, history and raw sources, `queue(kind)` replaces the eight batch and candidate fetchers, `import(from)` replaces five import tools, and `wiki_read`, `digest`, `server_status`, `reminders`, `saved_search` and `sync_status` absorb their neighbours.
- **Deprecated:** the 36 merged names. They are no longer listed but still work unchanged until 0.6.0, and are allowed exactly when the tool that replaced them is.
- **Added:** a `carded` outcome on `remind_me_resolve`, for a decision moved to a board card; it keeps its full weight. `server_status` reports the active tool profile, and a refused tool names the group that would enable it.
- **Changed:** tool descriptions lead with "remind-me" (core) or their group label, so clients that find tools by keyword search land on them.
- **Known limitations:** the default profile is still `full`; switching it to `core`, moving admin operations to the CLI and taking `self_update` off MCP is the next step. Per-tool call counts remain in memory only.
