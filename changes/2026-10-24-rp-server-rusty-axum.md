---
category: Changed
changelog: "`rp-server` serves on `rusty_axum` and `rusty_tokio` instead of axum, tower-http and `rusty_mcp_axum` (#875)."
---
## 2026-10-24 - rp-server leaves axum

- **Changed:** `rp-server` is a `rusty_axum` app. It serves on a `rusty_tokio` runtime of its own (`rp_server::serve`, which `main` and the integration tests both use). Its handlers still call tokio libraries (reqwest via `rp-router`), so `rusty_axum_tokio::TokioBridge` is the outermost layer and `main` keeps its tokio runtime for them. The MCP endpoint is `rusty_mcp_server::Mount` behind the same `check_auth` (`routes::mcp`). CORS is `rusty_axum::cors`, and the body limit is the server's `max_body_bytes` (still `413`). tower-http's `TraceLayer` is replaced by a debug-level log line per request (method, path, status, latency).
- **Removed:** the `axum`, `tower-http` and `rusty_mcp_axum` dependencies of `rp-server`, and the root `tower-http` workspace entry, which had no other user.
- **Known limitations:** the tests still drive the MCP endpoint with the `rmcp` client (dev only); replacing that oracle is separate. `rusty_mcp_axum` stays until `remind_me_remote`, `adk-mcp` and `agentgateway` have moved.
