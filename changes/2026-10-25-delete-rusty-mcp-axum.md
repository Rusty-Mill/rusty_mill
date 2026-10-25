---
category: Removed
changelog: The rusty_mcp_axum crate is deleted; `rusty_mcp_server`'s `rusty-axum` feature (`Mount`) replaces it (#875).
---
## 2026-10-25 - rusty_mcp_axum removed

- **Removed:** `crates/libs/protocol/rusty_mcp_axum`, which mounted a `rusty_mcp_server` handler in an axum app. It had no users left: rp-server, remind_me_remote and adk-mcp use `rusty_mcp_server::Mount` on `rusty_axum`, agentgateway has its own tower `McpService`, and rusty-mcp's tests use a test-only adapter (#1004 to #1056).
- **Changed:** the root README crate table, `docs/WORKSPACE-MAP.md`, the lockfile and the `rusty_mcp` README no longer list or use it. Change logs and research notes that mention it are history and stay as they are. ADR-0002's update follows separately.
