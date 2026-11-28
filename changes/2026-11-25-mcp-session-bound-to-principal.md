---
category: Security
changelog: rusty_mcp_server binds each HTTP session to the principal that opened it; another principal, or none, gets 404 (Let Em Loose #1375).
---
## 2026-11-25 - MCP sessions are bound to their principal

- **Security:** `rusty_mcp_server`'s Streamable HTTP sessions remember who opened them: `iss` and `sub` of the host's verified claims, else the whole claims. A request, `GET` stream or `DELETE` naming the session under any other principal, or with none, gets the same `404 unknown or ended session` as an unknown id, so a leaked session id is useless without its owner's token. A refreshed token for the same subject keeps the session. Sessions opened without auth are unchanged. Closes #727 gap 8 (card #1375, design D180).
- **Known limitations:** resumable stream ids (`Last-Event-ID`) are random and not yet bound to a principal.
