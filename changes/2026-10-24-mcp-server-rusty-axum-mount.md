---
category: Added
changelog: "`rusty_mcp_server` feature `rusty-axum` adds `Mount`, which serves the Streamable HTTP handler inside a `rusty_axum` app and replaces `rusty_mcp_axum` for its consumers (#875)."
---
## 2026-10-24 - rusty_mcp_server: the MCP handler inside rusty_axum

- **Added:** feature `rusty-axum` (off by default) and `mount::Mount`. `Mount::new(handler)` answers a `rusty_axum::Request` with `call(req)`, so an app routes it with `any(move |req| mcp.call(req))` or `fallback`. `at(path)` treats every request as addressed to that path (the old `router_at`), and `call_as(req, principal)` passes the caller to `Caller::principal` (the old `router_with_principal`). The handler runs on a `rusty_tokio` blocking thread; streamed replies are pumped into the response body, and a client that hangs up ends the pump and the handler's work.
- **Added:** `rusty_axum::Stream`, the `futures-core` trait `Body::from_stream` takes, re-exported so consumers need no direct dependency on it.
- **Changed:** `rusty_mcp_server` tests the mount over real sockets with `rusty_mcp_client_native` (a new dev-dependency), the same six cases as `rusty_mcp_axum`'s tests.
- **Known limitations:** the request body limit is the server's (`ServeOptions::max_body_bytes`), not a per-mount one. `rusty_mcp_axum` stays until its four consumers have moved.
