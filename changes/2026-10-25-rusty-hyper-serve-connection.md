---
category: Added
changelog: rusty_hyper::serve_connection serves HTTP/2 on one accepted connection, for a server with its own accept loop (#885).
---
## 2026-10-25 - rusty_hyper serves a single connection

- **Added:** `rusty_hyper::serve_connection(io, h2, deadline, stop, handler)`: the per-connection loop of `serve` (preface deadline, a task per call, GOAWAY and drain when `stop` completes), public so that `rusty_axum` can serve HTTP/2 on connections it accepted itself once TLS negotiates ALPN `h2` (#885). `serve` runs on it, unchanged.
