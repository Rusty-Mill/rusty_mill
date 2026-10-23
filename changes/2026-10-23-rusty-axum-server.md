---
category: Added
changelog: rusty_axum's HTTP/1.1 server on rusty_tokio and rusty_tls (#879 stage 2), with keep-alive, limits and graceful shutdown.
---
## 2026-10-23 - rusty_axum server: HTTP/1.1 on rusty_tokio and rusty_tls

- **Added:** `rusty_axum::serve(listener, tls, options, router)` returns a `Server` with `local_addr`, `shutdown` and `shutdown_on(signal)` (axum's `with_graceful_shutdown`). Each connection is one task that serves requests in order (keep-alive). It answers `Expect: 100-continue`, and closes on HTTP/1.0 or `Connection: close`. `ServeOptions` bounds connections (1024; more are closed unserved), head size (8 KiB, 431), body size (2 MiB, 413), idle time and per-read/write time (60 s) and the TLS handshake (10 s). Malformed requests get 400. Whole bodies are sent with `Content-Length`; streams are chunked and flushed per chunk, so SSE goes out as produced. A stream that fails is truncated (no terminating chunk). Shutdown stops accepting, closes idle connections, lets requests in flight finish and ends streamed bodies with their terminating chunk. The layout follows `rusty_hyper::serve`, but the stop signal is its own: `rusty_tokio`'s `watch::changed` keeps a cancelled waiter's waker until the next send, which would grow with every accept, request and chunk, so a waiter here unregisters when dropped.
- **Changed:** `rusty_http`'s `AsyncTransport` gains `get_mut`, so the server can flush a TLS stream after writing.
- **Known limitations:** HTTP/1.1 only. No SSE keep-alive comments yet, and no peer address for handlers (`ConnectInfo`), which `rp-server` and `remind_me_remote` will need. A handler that never finishes holds shutdown, as in `rusty_hyper`.
