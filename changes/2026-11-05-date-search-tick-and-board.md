---
category: Added
changelog: rusty_tick `/search` and `search_tasks` take created/updated date ranges; Mill Board `board_list` filters by created, updated and moved dates, moves read from rusty_tick's move history.
---
## 2026-11-05 - Date search in rusty_tick and Mill Board

- **Added:** rusty_tick `GET /api/v1/search` takes optional `createdFrom`, `createdTo`, `updatedFrom` and `updatedTo` (Unix ms, `[from, to)`); with dates alone it lists every task in range, trash excluded. The `search_tasks` MCP tool passes them through, and `query` is now optional.
- **Added:** Mill Board `board_list` takes `created_from`/`created_to`, `updated_from`/`updated_to` and `moved_from`/`moved_to`. "Moved during" reads rusty_tick's durable `GET /moves` history through `TickStore`, so it survives restarts; the in-memory store answers from its event log. Card #744.
- **Known limitations:** no date controls in either web UI yet; both search the data already loaded in the browser and get them in a follow-up. The board must run against a rusty_tick with `/moves` (#1278, deployed since 2026-10-31).
