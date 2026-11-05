---
category: Added
changelog: agentgateway gains an MCP test over a classic 2025-11-25 initialize session, alongside the stateless default.
---
## 2026-11-04 - Gateway MCP smoke test over a classic session

- **Added:** `mcp_rules::a_classic_session_carries_the_principal_to_every_call` pins the client to
  the `2025-11-25` revision, so it runs an `initialize` handshake and an `Mcp-Session-Id` session
  instead of the stateless revision the other suites negotiate. With a verified bearer token it
  checks two allowed calls and one denied call within that session (card #731, from the review of
  #1475). `tests/common/mcp.rs` gains `connect_classic`.
