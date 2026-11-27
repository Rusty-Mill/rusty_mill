---
category: Changed
changelog: rusty_agent_gateway gives each MCP target a `mode` (pooled, per_credential, per_session; default per_session) and `connections`, and closes per-session upstreams when the session ends; rusty_mcp_server adds `ToolSource::session_ended` (Let Em Loose #1377, D223).
---
## 2026-11-27 - Gateway: per-target upstream modes with session teardown

- **Changed:** an `mcp` target takes `mode: pooled | per_credential | per_session` and `connections: N` (default 8, today's constant). `per_session` gives each downstream session and token principal its own upstream connection or child process. A target with no `mode` is now `per_session` (LEL D102), and `--check` names each one. Set `mode: pooled` to keep the old sharing for a stateless server.
- **Added:** `rusty_mcp_server::ToolSource::session_ended(session)`, a default no-op the HTTP layer calls when a classic session ends by `DELETE` or by going idle. The gateway's federation closes that session's per-session upstreams there, which kills their stdio children.
- **Added:** `rusty_mcp_server::Caller::session`, the classic session the server validated for a request (`None` when stateless). The gateway keys `per_session` upstreams on it, not on the raw `Mcp-Session-Id` header, which the server ignores on stateless requests. A call admitted before its session ended opens no new upstream afterwards, calls queued on a closed upstream are refused, and its running call's connection is dropped when that call returns. An upstream a call still holds is not closed for being idle, and idle upstreams close on a reaper timer, not only on the next call.
- **Changed:** the `mock_mcp_server` fixture appends its pid when `MOCK_ECHO_PID` is set.
- **Known limitations:** only calls, prompt gets and resource reads are keyed; listings use the first connection. Stateless clients carry no session id, so their per-session upstreams close after 15 minutes idle or when the token's principal changes. Revoked tokens close their sessions in #1382's revocation slice. Approved by Nano as part of D223.
