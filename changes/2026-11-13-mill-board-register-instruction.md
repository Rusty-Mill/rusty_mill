---
category: Changed
changelog: Mill Board MCP instructions tell connector sessions to call agent_register first and pass agent_id and agent_key on writes.
---
## 2026-11-13 - Server instructions mention agent_register
So chat and voice sessions, which do not read AGENTS.md, register before writing.
