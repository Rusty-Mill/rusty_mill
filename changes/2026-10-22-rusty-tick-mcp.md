---
category: Added
changelog: rusty_tick serves MCP at /mcp, so an agent such as Claude Code can list, search, add, update, complete and delete your tasks under your API token.
---
## 2026-10-22 - rusty_tick: an MCP server for tasks

- **Added:** the server speaks MCP over Streamable HTTP at `/mcp`, built on the monorepo's `rusty_mcp_server`, with no new third-party dependencies. It runs in the same process and on the same port as the API. The data directory is locked by the running server, so a separate MCP process could not open it.
- **Added:** ten tools: `list_lists`, `list_tasks`, `due_tasks`, `search_tasks`, `get_task`, `add_task`, `update_task`, `complete_task`, `reopen_task` and `delete_task`. Each one runs the matching API route in-process with the caller's own token, so the API's validation, per-user stores and errors apply unchanged. Ids are checked to be UUIDs before they go into a path.
- **Security:** `/mcp` refuses a missing or wrong token with the same `401` as `/api`, before MCP sees the request. It accepts no browser `Origin`. It accepts any `Host`, since every call needs a bearer token.
- **Changed:** `rusty_tick::server::bind` serves the whole surface (API, web UI and MCP); the binary uses it. MCP requests do not hold the backend's lock while they wait; each tool call takes it for its one API request.
