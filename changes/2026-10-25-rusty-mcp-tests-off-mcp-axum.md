---
category: Changed
changelog: rusty-mcp's integration tests mount the MCP handler through a test-only adapter instead of rusty_mcp_axum (#875).
---
## 2026-10-25 - rusty-mcp tests without rusty_mcp_axum

- **Changed:** `tests/support/mcp.rs` mounts the blocking `rusty_mcp_server` handler in an axum router as `rusty_mcp_axum` did: same 1 MiB body limit, path, `VerifiedToken` principal, default headers and streamed replies. The tower layers are still tested in an axum stack.
- **Changed:** the `rusty_mcp_axum` dev-dependency is removed (`http-body`, `rusty_serve` and tokio `sync` added, all already in the workspace). axum stays a dev-dependency until the layers' tests move to another stack. No shipped code changes; one doc comment in `auth` no longer names `rusty_mcp_axum`.
