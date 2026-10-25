---
category: Changed
changelog: "`remind_me_remote` serves on `rusty_axum` and `rusty_tokio` instead of axum, tokio and `rusty_mcp_axum`; `rusty_axum` gains a `Form` extractor (#875)."
---
## 2026-10-25 - remind_me_remote leaves axum

- **Changed:** `remind_me_remote` is a `rusty_axum` app on a `rusty_tokio` runtime of its own (`run_blocking`, which `remind_me_cli` calls, is unchanged). The MCP endpoint is `rusty_mcp_server::Mount` pinned at `/mcp` as the router's fallback. The secret-path gate is still a layer in front of routing, and the OAuth `/mcp` bearer check wraps only that handler. `serve(listener, router)` takes a `rusty_tokio` listener and returns the running `Server`. The 4 MiB body limit is the server's `max_body_bytes`, so it now covers the OAuth forms too (`413`).
- **Added:** `rusty_axum::Form`, an `application/x-www-form-urlencoded` body decoded as `Query` decodes a query string: `415` for another content type and `422` for the wrong shape, as axum answers.
- **Removed:** the `axum`, `tokio` and `rusty_mcp_axum` dependencies of `remind_me_remote`. `tokio` stays a dev-dependency for the tests' reqwest client.
- **Known limitations:** `rusty_mcp_axum` stays until `adk-mcp` and `agentgateway` have moved.
