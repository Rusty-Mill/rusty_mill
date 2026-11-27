---
category: Changed
changelog: rusty_mcp_server passes the session owner it already derived to dispatch and tests that another principal can't open a session's stream (#1877 follow-up).
---
## 2026-11-27 - MCP session owner: review follow-ups

- **Changed:** `dispatch` takes the owner derived once per request instead of recomputing it when opening a session. The docs note that a token without `iss` counts as an empty issuer.
- **Added:** a test that a second principal holding the session id gets refused on the session's GET stream (#1375, #1877 review).
