---
category: Added
changelog: rusty_axum core, the in-house axum replacement (#879 stage 2): router, extractors, responses, SSE and in-process calls on rusty_http's types, runtime-free.
---
## 2026-10-23 - rusty_axum core: router, extractors, responses, SSE

- **Added:** `rusty_axum` (`crates/libs/net/rusty_axum`), the first stage 2 piece of #879. It covers the surface the `rk-app` pilot uses, with axum's names, so a consumer moves mostly by changing imports: `Router` with `{param}` paths (static segments win), `get`/`post`/`put`/`patch`/`delete`, `merge`, `with_state`; extractors `State`, `Query`, `Path`, `Json`, `HeaderMap`, `Method`, `String`, `Vec<u8>`, `Request` and a public `FromRequest` trait for custom ones; `IntoResponse` for status codes, text, `Html`, `Json`, `Result` and `(StatusCode, R)`; SSE (`Sse`, `Event`) as a streamed body; `Router::call` to answer a request in process (replaces `tower::ServiceExt::oneshot` in tests).
- **Design:** the core has no runtime and no `tokio`. Request bodies arrive whole (the server bounds them), so extraction is synchronous. Types are `rusty_http`'s, not the `http` crate's. New third-party dependency: `futures-core` (the `Stream` trait only, already in the lockfile).
- **Known limitations:** no server yet (next PR: the HTTP/1.1 driver on `rusty_tokio`/`rusty_tls`), no middleware or CORS, no SSE keep-alive (needs a timer, which belongs to the driver), no response header tuples. No consumer is moved.
