# rusty_axum

The in-house replacement for `axum` (#879, `docs/research/RUSTY-AXUM-SCOPE.md`).
Names follow axum's, so moving a consumer is mostly a change of imports.

```rust
use rusty_axum::routing::{get, post};
use rusty_axum::{Json, Router, State};

let app = Router::new()
    .route("/health", get(|| async { "ok" }))
    .route("/echo", post(|Json(v): Json<serde_json::Value>| async move { Json(v) }))
    .with_state(());
let response = app.call(rusty_axum::Request::new(rusty_axum::Method::Get, "/health")).await;
```

- Runtime-free core: routing, extraction and responses never touch a socket.
  `Router::call` answers a request in process, which is how handlers are tested.
- Request bodies arrive whole, so extractors are synchronous (`FromRequest`).
  Response bodies may stream (`Body::from_stream`, `Sse`).
- `Method`, `StatusCode` and `HeaderMap` come from `rusty_http`.
- `routing::any` takes every method a path has no handler of its own for, and
  `Router::fallback` every unmatched path (the shape an MCP mount needs).
- `Form` decodes an `application/x-www-form-urlencoded` body as `Query` decodes
  a query string (415 for another type, 422 for the wrong shape, as axum).
- `ConnectInfo` extracts the client's address, which the server sets on every
  request (`Request::with_peer_addr` in tests).
- `Sse::keep_alive(KeepAlive::new())` sends a comment after each quiet
  interval; such a stream is polled inside `rusty_tokio`, as the server does.

- `serve(listener, tls, options, router)` runs HTTP/1.1 on `rusty_tokio`, plain
  or over `rusty_tls`: keep-alive, `Expect: 100-continue`, bounded head, body,
  idle time and connections, and graceful shutdown that ends SSE streams cleanly.
  With the `http2` feature and `ServeOptions.http2`, a TLS connection that
  negotiates `h2` over ALPN is served as HTTP/2 (no h2c, no upgrades).
- `serve_with(listener, accept, options, router)` hands each accepted socket to
  an `Accept` first, which returns it plain, as a finished TLS stream, or as
  taken over (`Accepted::Done`). That is where a gateway picks a certificate by
  SNI or forwards a TLS passthrough port untouched. It runs on the connection's
  own task, within `handshake_timeout`.
- `Admission` (returned by `Accept::admission`) sheds load before a body is
  read: at most `with_max_in_flight(n)` requests from head to response head,
  the rest `503` with `Retry-After: 1`, and `with_timeout(d)` answers `504` to
  one whose body and response are not done in time. Clones share the count.
  Middleware cannot do this, since bodies are read whole before routing.

- `Router::layer` wraps every request: `middleware::from_fn(|req, next| ..)` or
  `cors::Cors` (tower-http's `CorsLayer` behaviour). There is no `route_layer`
  or `nest`: guard one route by wrapping its handler, and route a mounted
  service with `any` or `fallback`.

Status: `rk-app` (#948), `rp-server`, `remind_me_remote` (#875) and `agentgateway` (#885) have moved.
