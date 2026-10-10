---
category: Fixed
changelog: **`rusty-mcp-client`: a classic-only HTTP server is reachable again.** `Link` did not forward `clear_protocol_version`, so after a refused `server/discover` the `initialize` fallback still sent `MCP-Protocol-Version: 2026-07-28` and a classic-only server answered `-32022`.
---
## 2026-10-10 - rusty-mcp-client: a classic-only HTTP server is reachable again

- **Fixed:** `Link` in `rusty-mcp-client` forwarded `set_protocol_version` but not `clear_protocol_version`, so the default no-op ran. After a refused `server/discover` the `initialize` fallback still carried `MCP-Protocol-Version: 2026-07-28`, and a server that speaks only a classic revision rejected it with `-32022`. The client could not connect to any such server. `agentgateway-mcp`'s own `Link` already forwarded it; this copy did not. Found by the red CI on #577: `rusty-mcp-client::end_to_end::a_session_the_server_forgot_counts_as_dead` and `rp-mcp`'s `a_restarted_upstream_is_reconnected_by_the_supervisor`.
- **Tested:** new `a_server_that_speaks_only_a_classic_revision_is_reachable`; with the forward removed it and the two tests above fail.
