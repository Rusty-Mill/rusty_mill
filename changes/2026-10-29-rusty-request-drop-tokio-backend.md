---
category: Removed
changelog: rusty_request's `tokio` feature and `Body::streaming_tokio`, and rusty-mcp-client's `tokio` feature, are removed; tokio programs call `rusty_tokio::runtime::install_fallback()` instead.
---
## 2026-10-29 - rusty_request drops its real-tokio backend

- **Removed (breaking):** `rusty_request`'s `tokio` feature, its per-call
  runtime choice (`src/rt.rs`), the `src/tokio_compat.rs` shim and
  `Body::streaming_tokio`. Requests always run on `rusty_tokio`; a program
  whose tasks run on tokio calls `rusty_tokio::runtime::install_fallback()`
  once at startup.
- **Removed (breaking):** `rusty-mcp-client`'s `tokio` feature, which only
  forwarded `rusty_request/tokio`. `adk-mcp` (`client`), `rk-mcp` (`rmcp`)
  and `rp-mcp` no longer turn it on.
- **Changed:** `rp-server` and `rk-app` (with `mcp-server`) call
  `install_fallback()` at the start of `main`, so the MCP gateway's OAuth
  token fetch works from their tokio runtime.
- **Changed:** `rusty_request`'s `tests/tokio_feature.rs` became
  `tests/foreign_executor.rs`: plain and TLS requests, pooled reuse and
  redial from `#[tokio::test]` with the fallback installed.
