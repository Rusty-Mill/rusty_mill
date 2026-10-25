---
category: Added
changelog: "`rusty_axum` gains `routing::any`, `Router::fallback`, the `ConnectInfo` peer-address extractor and SSE keep-alive, which `rp-server` needs to leave axum (#875)."
---
## 2026-10-24 - rusty_axum: any, fallback, peer address, SSE keep-alive

- **Added:** `routing::any` (every method a path has no handler of its own for) and `Router::fallback` (every unmatched path; `merge` takes the other router's, and refuses two). Together they mount a service such as the MCP handler at a path or at the root.
- **Added:** `ConnectInfo`, the client's `SocketAddr`. The server sets it on every request; `Request::with_peer_addr` and `Request::peer_addr` set and read it in process. Without one the extractor answers 500.
- **Added:** `Sse::keep_alive(KeepAlive)` sends a comment (empty, every 15 s by default) whenever the stream has been quiet for the interval. The timer starts on the first quiet poll, so such a stream must be polled inside `rusty_tokio`, which the server does.
- **Known limitations:** no `nest` or `route_layer`. A route-scoped guard wraps that route's handler instead. These are the rusty_axum gaps `rp-server` hits (#875); the MCP mount and the `rp-server` move follow in their own PRs.
