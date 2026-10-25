---
category: Changed
changelog: agentgateway serves its MCP routes through a local tower service instead of rusty_mcp_axum, and its shipped binary no longer depends on axum (#875).
---
## 2026-10-25 - agentgateway without rusty_mcp_axum

- **Changed:** the MCP backend is `mcp_service::McpService`, a tower service on the gateway's own tokio runtime. It hands each request to the blocking `rusty_mcp_server` handler, which is what `rusty_mcp_axum::router_at_with_principal` did, with the same body limit, path, principal and headers. Responses use the gateway's own `body::Body` instead of `axum::body::Body`, so `Gateway::handle` now returns `http::Response<agentgateway::body::Body>`. Content types are unchanged.
- **Changed:** the `axum` and `rusty_mcp_axum` dependencies are removed (`rusty_serve`, `rusty_http` and tokio `sync` added). axum stays a dev-dependency, for the upstream servers in the integration tests, until those move. hyper and tower stay (#885, #886).
