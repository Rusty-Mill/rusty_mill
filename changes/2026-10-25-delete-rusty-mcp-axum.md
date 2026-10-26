---
category: Removed
changelog: Delete rusty_mcp_axum, the axum bridge for rusty_mcp_server; nothing depends on it any more (#875).
---
## 2026-10-25 - rusty_mcp_axum deleted

- **Removed:** `crates/libs/protocol/rusty_mcp_axum`, the bridge that mounted `rusty_mcp_server`'s blocking HTTP handler in a third-party axum app. Its consumers have moved: `rp-server`, `remind_me_remote` and `adk-mcp` serve on `rusty_axum` through `rusty_mcp_server`'s `Mount`, `agentgateway` through its own tower `McpService` (it is still on hyper and tower), and `rusty-mcp`'s tests through a test-only adapter.
- **Changed:** ADR-0002 records the bridge as retired, with `agentgateway`'s move onto `rusty_axum` and `rusty-mcp`'s tower and `http` exit still open (#885, #886); the root README, the `rusty_mcp` README and the async-runtime inventory no longer list the crate.
