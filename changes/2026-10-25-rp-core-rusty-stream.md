---
category: Changed
changelog: "`rp-core`'s `ChatStream` is a `rusty_stream` stream; `rp-core` and `rp-router` no longer depend on `futures` (#910)."
---
## 2026-10-25 - rusty_provider streams on rusty_stream

- **Changed:** `rp_core::ChatStream` is now `rusty_stream::BoxStream`, so `rp-core` has no async dependency and `rp-router` drops `futures` for `rusty_stream`. `rp-providers` adapts reqwest's SSE body and `rp-server` adapts the stream for `rusty_axum`'s `Sse`, both with `rusty_stream::Compat`. `rp-mcp`'s gateway lock is `rusty_tokio::sync::RwLock`.
- **Known limitations:** `rp-mcp` stays on tokio. Its tools block on `rp-router` (reqwest) and its gateway drives `rusty-mcp-client`'s tokio transport, both on the tokio runtime `rp_mcp::build` is called from. It moves with #906.
