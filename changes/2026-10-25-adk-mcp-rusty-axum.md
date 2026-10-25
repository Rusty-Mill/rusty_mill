---
category: Changed
changelog: adk-mcp serves streamable HTTP on rusty_axum; `router()` now returns `rusty_axum::Router` (breaking), and axum and rusty_mcp_axum are gone from its tree (#875).
---
## 2026-10-25 - adk-mcp on rusty_axum

- **Changed (breaking):** `adk_mcp::router` (and `rusty_adk::mcp::router`) returns a `rusty_axum::Router` instead of an `axum::Router`. Merge it with an app's other `rusty_axum` routes. Serve it on a `rusty_tokio` runtime. Build it inside a tokio runtime, as before: tool calls still run there.
- **Changed:** `serve_http` keeps its signature. It serves on a `rusty_axum` server on its own `rusty_tokio` runtime, which runs on a blocking thread of the caller's tokio runtime, so dropping its future no longer stops the server. The `http` feature drops axum, rusty_mcp_axum and `tokio/net`, and adds rusty_axum and rusty_tokio. This moves one more consumer off rusty_mcp_axum (#875).
- **Known limitations:** HTTP/1.1 only (no h2c). A path other than `/` matches exactly, so subpaths below it are 404 instead of being handed to the MCP handler.
