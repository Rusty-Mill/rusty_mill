---
category: Fixed
changelog: rusty_axum's HTTP/1 server drops a running handler when the client hangs up, as hyper does.
---
## 2026-11-03 - rusty_axum notices an HTTP/1 client hang-up

- **Fixed:** a long-running handler kept going after its HTTP/1 client disconnected (card
  #408). While a handler runs, the server now watches the connection and drops the handler
  future when the client closes, as hyper does. Bytes the client sends meanwhile (a pipelined
  request) are read ahead and kept for the next request, up to `max_head_bytes`.
- **Added:** `rusty_http::async_tokio::AsyncTransport::closed`, the read-ahead wait this uses.
- **Known limitations:** as on hyper's default, a client that half-closes its write side after
  sending a request is treated as gone unless the response is ready at once. HTTP/2 is unchanged.
