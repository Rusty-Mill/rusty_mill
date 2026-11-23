---
category: Added
changelog: Mill Board's card_add and card_update take the typed fields acceptance, scope, why and summary; they round-trip through every store and the tick-to-SQLite migration.
---
## 2026-11-19 - Typed card fields: MCP params (#1580)

- **Added:** `card_add` and `card_update` accept `acceptance` (1,200 B), `scope` (800), `why` (600) and `summary` (320), refused over their cap before anything is written; `""` clears a field on update. `card_take` and `card_get` return them; `board_list` summaries leave them out. The tool descriptions tell agents to use the fields instead of `##` headers in the body.
- **Added:** tests that the fields round-trip on the memory, SQLite and tick stores, and that the tick-to-SQLite migration copies them exactly.
- **Known limitations:** no UI yet (#1581). `summary` is not in the summary view: the tick adapter reads no documents there, so it can follow once the tick adapter is removed. The migration of headed body sections (#1582) is separate.
