---
category: Changed
changelog: rk-app's web gateway runs on rusty_axum and rusty_tokio instead of axum, tower and tower-http (#879 pilot).
---
## 2026-10-23 - rk-app gateway on rusty_axum

- **Changed:** `rusty-keys --gateway` now serves HTTP/SSE with `rusty_axum` on its own `rusty_tokio` runtime. The `gateway` feature no longer pulls in `axum` 0.7, `tower` or `tower-http`. The routes, status codes, bearer auth, CORS policy (exact origin, GET/POST, three headers) and SSE frame names are unchanged.
- **Changed:** `Gateway::router` takes the `tokio::runtime::Handle` that its sessions run on. The new `Gateway::serve` runs until its future is dropped: cancelling it (a timeout, a select, an abort) shuts the server down and releases the listener. `RUSTYKEYS_GATEWAY_CORS_ORIGIN=*` still allows any origin. The session is still tokio-based, so the router's outermost layer is `rusty_axum_tokio::TokioBridge`.
- **Known limitations:** while rk-app's session code is on tokio, two runtimes run in the gateway process. The bridge goes away when that code moves to `rusty_tokio`.
