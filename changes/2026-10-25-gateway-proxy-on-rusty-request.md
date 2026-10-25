---
category: Changed
changelog: agentgateway's proxy client moves off hyper onto rusty_request; proxied request bodies are read whole within limits.maxRequestBodyBytes (10 MiB) and can always be retried (#885).
---
## 2026-10-25 - agentgateway: the proxy client is rusty_request

- **Changed:** `host`, `service` and `dynamic` backends are dialled with `rusty_request` instead of hyper's legacy client, so `agentgateway-proxy` no longer depends on hyper, hyper-util or http-body-util (#885). Upstream connections are still pooled (30 s idle), responses still stream back as they arrive, and only a failure before the request was sent is retried without a listed status.
- **Changed:** a request to a proxied backend is read whole before it is forwarded, within the new `config.limits.maxRequestBodyBytes` (10 MiB unless set); a larger body is refused with `413`. Before, bodies streamed through with no limit, and only those with a `Content-Length` of at most 64 KiB could be retried. Now every forwarded body, chunked included, is retried as the route's `retry` policy says. The owner chose buffering over streaming request bodies on 2026-10-25.
- **Changed:** `agentgateway_proxy::RequestBody`, `MAX_REPLAY_BYTES` and `is_replayable` are gone: `HostProxy::proxy` takes a `Request<Bytes>`, and the request body type moves to `agentgateway::body::RequestBody`.
- **Known limitations:** the gateway's server is still hyper on tokio; `rusty_request` runs on tokio through its `tokio` feature until the gateway moves to rusty_axum. Upstreams are reached over HTTP/1.1 only, as before. A request with no `User-Agent` or `Accept` reaches the upstream with `rusty_request`'s defaults. A request header that is not valid text is dropped (logged at debug).
