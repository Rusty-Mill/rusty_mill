---
category: Added
changelog: rusty_axum_tokio, the migration bridge for rusty_axum consumers still on tokio libraries (#879 stage 2).
---
## 2026-10-23 - rusty_axum_tokio bridge

- **Added:** `rusty_axum_tokio::TokioBridge`, a `rusty_axum` middleware that enters a `tokio` runtime on every poll, and on drop, of the handler future and of a streamed response body. Drop is covered because tokio resources such as a pooled connection spawn their cleanup when dropped, and a request can end by cancellation, a HEAD request's suppressed body, or shutdown. Handlers that still call `reqwest`, `sqlx`, `tonic` or `tokio::time` can then run on `rusty_tokio`. Add it as the router's last layer. The consumer keeps a multi-threaded tokio runtime running. The crate is deleted when its last consumer leaves tokio (owner decision, 2026-10-23).
- **Added:** `rusty_axum::BodyStream` and `Body::map_stream`, so a layer can wrap a streamed body.
