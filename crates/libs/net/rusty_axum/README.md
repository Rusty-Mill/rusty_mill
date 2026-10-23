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

- `serve(listener, tls, options, router)` runs HTTP/1.1 on `rusty_tokio`, plain
  or over `rusty_tls`: keep-alive, `Expect: 100-continue`, bounded head, body,
  idle time and connections, and graceful shutdown that ends SSE streams cleanly.

Status: middleware and CORS come next; no consumer has moved yet.
