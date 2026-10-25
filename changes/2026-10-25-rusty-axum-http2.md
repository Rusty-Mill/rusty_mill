---
category: Added
changelog: rusty_axum serves HTTP/2 next to HTTP/1.1 on a TLS port that negotiates ALPN h2 (feature http2, #885).
---
## 2026-10-25 - rusty_axum speaks HTTP/2 over TLS

- **Added:** feature `http2` and `ServeOptions::http2` (off by default). A TLS connection that negotiates ALPN `h2` is served as HTTP/2 by `rusty_hyper::serve_connection`; every other connection stays HTTP/1.1, so one port serves both, as `agentgateway` needs to leave hyper (#885). Each stream is buffered into the same `Request` (peer address, `:authority` as `Host`, split cookies joined) within the same body and idle limits, and answered by the same `Router`. Streamed bodies (SSE) and HEAD behave as on HTTP/1.1; a client that resets the stream drops the handler; shutdown sends GOAWAY, ends streamed bodies cleanly and lets requests in flight finish.
- **Added:** the HTTP/1.1 limits carry over to HTTP/2. `max_head_bytes` bounds the decoded header list (431), and is advertised as `SETTINGS_MAX_HEADER_LIST_SIZE`. `idle_timeout` bounds each body read, each write blocked on the client's flow-control window (the stream is reset, so a client that stops reading cannot hold a response or a shutdown open), and a connection with no open stream (closed with GOAWAY). A connection has at most 100 streams open (`SETTINGS_MAX_CONCURRENT_STREAMS`). A request whose `Host` differs from `:authority`, or that repeats `Host`, is refused with 400.
- **Known limitations:** no cleartext h2c, no connection upgrades, request trailers are dropped. The acceptor must offer `h2` (usually with `http/1.1`) for the feature to do anything.
