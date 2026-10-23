---
category: Added
changelog: rusty_axum middleware (Router::layer, from_fn) and CORS (#879 stage 2).
---
## 2026-10-23 - rusty_axum middleware and CORS

- **Added:** `Router::layer` wraps every request of a router, including those answered 404 or 405. The last layer added runs first, as in axum. Middleware is the `Middleware` trait, usually written with `middleware::from_fn(|req, next| ..)` and `Next::run`. A layer can answer without calling the handler (an auth gate). `cors::Cors` behaves like `tower-http`'s `CorsLayer`: it answers preflights itself, echoes `Access-Control-Allow-Origin` only for allowed origins (`*` for `Allow::Any`) and sets `Vary`. Configuration is `Allow::Any` or an exact list for origins, methods and headers, plus `max_age`; `Cors::permissive()` allows everything.
- **Changed:** `Router` is now cheap to `Clone` (routes and layers are shared).
- **Known limitations:** no `route_layer`, no `from_fn_with_state` and no request extensions yet (`rp-server`, `rusty_acp` and `remind_me_remote` need them, and they come when those move). `merge` refuses a router that already has layers, so merge first, then layer.
