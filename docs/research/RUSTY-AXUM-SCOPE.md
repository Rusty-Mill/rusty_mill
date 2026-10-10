# `rusty_axum`: what an in-house web framework would take

Status: scoping only, no code. Written 2026-10-09 after `rusty_mcp_axum` (the
bridge that lets `rp-server` and `adk-mcp` keep axum while the MCP crates stop
depending on it) raised the question "can we make our own axum?". Answer:
yes, but it is a new framework, not an MCP step, and nothing forces it yet.

## 1. Who uses axum today

Crates with `axum` in their manifest (13, counted by manifest, not by
effort): `rusty_a2a`, `rusty_acp`, `rusty-mcp`, `rusty_mcp_axum`,
`rusty_mcp_client_native` (dev), `adk-a2a`, `adk-mcp`, `a2a-agent-server`
(example), `agentgateway`, `agentgateway-mcp`, `remind_me_remote`,
`rp-server`, `rk-app`, and `nexus-memory-hub` (**Nexus is frozen: out of
scope**). By a rough grep of `axum::` paths they use: `Router` with
`route` / `nest` / `fallback`, `serve`, the `State` extractor, `from_fn`
middleware, `IntoResponse`, `Body` and `to_bytes`, `http` header and status
types, and server-sent events (`rusty_a2a`). `rp-server` also uses
`tower-http` (CORS, tracing).

**Not verified:** the grep counts any `axum::` path, including tests; no crate
was read for what it truly depends on (extractor kinds, WebSocket, multipart,
`tower` layer stacks).

## 2. What exists in-house

- `rusty_serve`: a blocking, thread-per-connection HTTP/1.1 server with
  `Handler` / `SharedHandler`, `Limits`, streamed bodies, TLS via
  `rusty_tls`. Decided acceptable for MCP (plan, decision 3), not a web
  framework: no router, no extractors, no async.
- `rusty_http` (types, headers, methods, status), `rusty_h2`, `rusty_tokio`,
  `rusty_request` (client).
- Not checked: whether `rusty_tokio` can host a server accept loop for
  thousands of connections, which an async framework needs.

## 3. Pieces of a `rusty_axum`

1. **Async server driver** (accept loop, HTTP/1.1, optionally h2, graceful
   shutdown, body streaming) on `rusty_tokio` and `rusty_tls`. The plan
   deferred this "only if a forcing function appears".
2. **Router**: path matching with parameters, nesting, method routing,
   fallback.
3. **Handler model**: async functions with extractors (`State`, `Path`,
   `Query`, `Json`, headers, body) and `IntoResponse`. This is the part that
   makes axum feel like axum, and the largest design surface (trait-heavy).
4. **Middleware**: `from_fn`, plus the layers apps actually use (CORS,
   tracing, body limit, concurrency limit, timeout).
5. **SSE** (and WebSocket, only if some crate needs it).

## 4. Options

- **A. Keep axum, shim it** (done): `rusty_mcp_axum`. Zero risk, and the MCP
  crates are axum-free.
- **B. Thin framework over `rusty_serve`** (blocking): a router, a few
  extractors, `from_fn`-style middleware, all synchronous. Small (hundreds of
  lines), enough for apps whose handlers are mostly I/O-light. Does not
  replace axum where async handlers or many idle connections matter
  (`agentgateway` proxies streams; `rusty_a2a` has SSE).
- **C. Full async `rusty_axum`**: items 1 to 5. Weeks of work; each of the 13
  crates then migrates by hand. Only worth it if "no third-party web stack"
  is a goal in itself.

Recommendation: stay on A until a crate other than MCP is being de-axumed for
its own reasons; if one is, build B first and move the smallest consumer
(`rk-app`) to see what is missing, before committing to C.

## 5. Questions for the owner (not decided here)

1. Is dropping `axum` repo-wide a goal, or only from the MCP crates?
2. If repo-wide, is B (blocking) acceptable for any crate, or must it be async?
3. Does `rusty_tokio` stand in for `tokio` for servers, or are these crates
   expected to stay on `tokio` for now?
4. Which consumer should go first? (Suggested: `rk-app`, the smallest.)
