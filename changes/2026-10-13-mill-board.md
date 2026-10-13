---
category: Added
changelog: **`rusty_mill_board`**, a capability-gated kanban board served as an MCP server and a live web UI, stored in `rusty_multimodal_db_engine`.
---
## 2026-10-13 - Mill Board: kanban MCP server and web UI

- **Added:** `crates/apps/rusty_mill_board`. Cards move through backlog, ready, building, qa, review, done and blocked; each move needs a capability and leaving a worked column needs the claim. Ten MCP tools and `POST /api/<op>` come from one operation table. The UI is a single page with drag-and-drop that follows the event log by polling. Durable storage is `rusty_multimodal_db_engine` (`replace_if` gives atomic claims); `--store` omitted means in memory.
- **Tests:** 10 unit tests (domain rules, operation table), a scenario run against both stores including reopen, and three socket-level tests of the HTTP guard (host, bearer, JSON-only writes), status mapping, UI and MCP. Run by hand: restart persistence, and drag-and-drop plus live sync across two headless-Chromium pages.
- **Known limitations:** roles are trusted as claimed (the bearer token authenticates, not the role). The card write and its event are two engine writes, not one transaction. `events_since` scans the event rows, which is fine for a board and not for a long log. The UI polls every 1.5 s rather than streaming. GitHub sync and the autonomous runner are not built; they are meant to be clients of the tools (the runner through BBP). No real MCP client was run against it; only raw JSON-RPC over HTTP.
