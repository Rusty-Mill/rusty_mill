# rusty_mcp_axum

Mounts a [`rusty_mcp_server`] Streamable HTTP handler inside an [axum] app, for
programs that already serve other routes with axum (`rp-server`, `adk-mcp`).

```rust,ignore
let handler = Arc::new(HttpHandler::new(server, HttpConfig { path: "/mcp".into(), ..Default::default() }));
let app = Router::new().nest_service("/mcp", rusty_mcp_axum::router(handler, 1 << 20));
```

[`router`] answers every request itself, so layers (authentication, CORS) go on
the returned router before it is nested. The handler is blocking, so each
request runs on a blocking thread of the tokio runtime; an event-stream reply
is pumped from that thread into the response body, and a client that hangs up
ends it. Mount it under the path the handler's `HttpConfig::path` names: the
handler sees the original, unstripped request target.

This crate exists so an axum app can host MCP without the MCP crates depending
on axum. A program with no axum should use `rusty_mcp_server::bind_http`.
