# `rusty_axum`: what an in-house web framework would take

Status: scoping only, no code. Written 2026-10-09 after `rusty_mcp_axum` (the
bridge that lets `rp-server` and `adk-mcp` keep axum while the MCP crates stop
depending on it) raised the question "can we make our own axum?". Answer:
yes, but it is a new framework, not an MCP step, and nothing forces it yet.

**Superseded in part (2026-10-22):** the owner decided to build `rusty_axum`
as the full async framework (option C) and to drop `axum` repo-wide; see §5.
The recommendation in §4 is kept as history.

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

This paragraph was a grep. The crate-by-crate reading is in §6 (stage 1 of
#879, 2026-10-22).

## 2. What exists in-house

- `rusty_serve`: a blocking, thread-per-connection HTTP/1.1 server with
  `Handler` / `SharedHandler`, `Limits`, streamed bodies, TLS via
  `rusty_tls`. Decided acceptable for MCP (plan, decision 3), not a web
  framework: no router, no extractors, no async.
- `rusty_http` (types, headers, methods, status), `rusty_h2`, `rusty_tokio`,
  `rusty_request` (client).
- Whether `rusty_tokio` can host a server accept loop for thousands of
  connections: yes, measured in §6.2.

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

Recommendation (2026-10-09, superseded by the 2026-10-22 decision in §5):
stay on A until a crate other than MCP is being de-axumed for its own reasons; if one is, build B first and move the smallest consumer
(`rk-app`) to see what is missing, before committing to C.

## 5. Questions for the owner

**Decided 2026-10-22 (owner):** roll our own `rusty_axum`. Question 1: dropping
`axum` repo-wide is a goal. Question 2: the target is option C, a full async
framework, not B. Question 3: the intended base is `rusty_tokio` and
`rusty_tls`, but whether `rusty_tokio` can drive a server accept loop for
many connections is unverified; stage 1 of #879 answers it, and a "no" stops
the plan. Question 4 stays open: `rk-app` is the planned pilot, confirmed only
by the stage 1 consumer inventory. `rusty_mcp_axum` stays (Tier T, ADR-0002
Amendment 1) until its consumers have moved, `agentgateway` last.

1. Is dropping `axum` repo-wide a goal, or only from the MCP crates?
2. If repo-wide, is B (blocking) acceptable for any crate, or must it be async?
3. Does `rusty_tokio` stand in for `tokio` for servers, or are these crates
   expected to stay on `tokio` for now?
4. Which consumer should go first? (Suggested: `rk-app`, the smallest.)

## 6. Stage 1 findings (#879, 2026-10-22)

Read from the source of each consumer at `4fbc328b`, not from a grep. Only
non-test code counts as a runtime need; test-only use is listed apart.

### 6.1 What each consumer uses

| Crate | Where axum is | Router | Extractors | Responses | Middleware, layers | Server |
|---|---|---|---|---|---|---|
| `rusty_a2a` (feature `server`) | src | `route` with `{tenant}/tasks/{id}`-style params, `get().post()`, `merge`, `with_state` | `State`, `HeaderMap`, `Bytes`, `Path`, `Query`, `Json`; own `FromRequest` / `FromRequestParts` that remap `JsonRejection` / `QueryRejection` | tuples of status + headers (`ETag`, `Cache-Control`) + body; SSE with `KeepAlive` from a `futures` `Stream`, `Last-Event-ID` | none | `axum::serve`. Public API returns `axum::Router` (`into_router`, `router`). gRPC is on `tonic`, not axum |
| `rusty_acp` (feature `server`) | src | 12 routes, params incl. `Path<(String, usize)>`, `with_state` | as `rusty_a2a`, plus `Extension`, own `FromRequest` (`AcpJson`) | `Json`, tuples, SSE with `KeepAlive` and event ids / names | `DefaultBodyLimit`, `from_fn` (trace span, inserts a request extension) | public API returns `axum::Router`; docs tell users to add their own middleware to it. Example uses `with_graceful_shutdown` |
| `rp-server` | src | 16 paths, `get` / `post` / `patch` / `delete`, `{name}` params, `nest_service` of the MCP router, `with_state` | `State`, `ConnectInfo`, `HeaderMap`, `Path`, `Query`, `Json`, `Bytes`, `Request` | `Json`, tuples, `Retry-After` headers, SSE with `KeepAlive` (chat streaming) | `tower-http` `TraceLayer` and `CorsLayer` (permissive or an origin list), `DefaultBodyLimit`, `from_fn_with_state` (concurrency limit, MCP auth) | `axum::serve` with `into_make_service_with_connect_info` |
| `remind_me_remote` | src | `route`, `merge`, `nest_service` of the MCP router, `layer` vs `route_layer` | `State`, `Query`, `Form`, `Json`, `Request`, `ConnectInfo` (read, see 6.4) | `Html`, `Json`, tuples | `from_fn_with_state` (secret gate, OAuth gate) | `axum::serve` |
| `rk-app` (feature `gateway`, axum **0.7**) | src | 9 routes, `get` / `post`, generic `State<Arc<Gateway<M>>>` | `State`, `Query<HashMap>`, `HeaderMap`, `Json` | `Json`, tuples, SSE (event names and ids, no keep-alive) from a channel | `tower-http` `CorsLayer` (one exact origin) | `axum::serve`; tests call the router in-process (`tower::ServiceExt::oneshot`) |
| `adk-mcp` (feature `http`) | src | `fallback`, `fallback_service` / `nest_service` of the MCP router | none | a 500 tuple | none | `axum::serve`; public `router() -> axum::Router` |
| `rusty_mcp_axum` | src | `fallback` | `Request`, `OriginalUri`; `to_bytes` with a limit | `Body::from_stream` (streamed MCP replies), `spawn_blocking` into the blocking handler | none | none. Deleted by #875, not ported |
| `agentgateway` | src | **not an axum app**: own router (`agentgateway-core`) | none | axum `Body` / `Response` / `IntoResponse` as its response type | `rusty-mcp`'s tower `LimitsLayer` and `McpMetricsLayer` | **`hyper-util` auto (h1 + h2) with `serve_connection_with_upgrades`, graceful shutdown, own TLS terminator**. axum's only real role: the MCP backend is a `rusty_mcp_axum` router |
| `rusty-mcp` | dev only | tests mount the MCP router | | | its lib exports tower layers over `http::Request`, used by `agentgateway`; no axum in `src` | |
| `agentgateway` tests (9 files) | dev only | mock upstreams: `Router::new().fallback(any(...))` | `Request`, `Json`, `to_bytes` | `Response::builder` | `from_fn` | `serve`, `with_graceful_shutdown`; `tls.rs` needs h2 |
| `agentgateway-mcp` | dev only | a scripted gRPC ext-proc upstream, `fallback(any(...))` | `Request` | | | needs **h2**, graceful shutdown |
| `rusty_mcp_client_native` | dev only | `nest_service("/mcp", rmcp's StreamableHttpService)` | | | | hosts the third-party `rmcp` server as a wire-format oracle |
| `adk-a2a`, `a2a-agent-server` (example) | dev only / example | none: `axum::serve(listener, rusty_a2a router)` | | | | moves with `rusty_a2a` |

Not used by any consumer: **WebSocket**, multipart, cookies, `TypedHeader`,
`MatchedPath`, HTTP/2 for an axum app (h2 appears only in `agentgateway`'s
hyper server and two test mocks).

The union a `rusty_axum` must cover, by stage:

- **Pilot (`rk-app`):** router with method routing, `State`, `Query`,
  `HeaderMap`, `Json`, SSE from a channel with event names and ids
  (`src/gateway.rs`, `stream`), a CORS layer, `serve`, and calling
  the router in-process without a socket (for its tests).
- **Middleware + SSE (`rusty_a2a`, `rp-server`):** path parameters (single,
  several, tuple), `merge`, `nest_service` / `fallback`, `Path`, `Bytes`,
  `ConnectInfo`, `Extension`, user-defined extractors with rejection mapping,
  `from_fn` and `from_fn_with_state`, `layer` vs `route_layer`, a body limit,
  a request trace span, CORS (permissive, list, exact), SSE keep-alive,
  streamed bodies.
- **Rest:** `Form`, `Html`, `OriginalUri`, graceful shutdown.

### 6.2 Can `rusty_tokio` drive a server accept loop for many connections?

**Yes.** Measured, not inferred. A throwaway probe (not committed) on
`rusty_tokio` 0.2.1, release build, Linux, 4 vCPUs, loopback: the server is a
`Builder::new_multi_thread().worker_threads(2)` runtime running
`TcpListener::accept` in a loop and spawning one task per connection that
answers HTTP/1.1 keep-alive requests; the client is a second 2-worker
`rusty_tokio` runtime in the same process that opens N connections, holds
them all open and idle at a barrier, then sends R requests on each.

| N connections | R per connection | All N open after | Server live at once | Requests OK | Wall time | OS threads (whole process) |
|---|---|---|---|---|---|---|
| 1,000 | 50 | 128 ms | 1,000 | 50,000 / 50,000 | 0.58 s | 9, constant |
| 8,000 | 20 | 382 ms | 8,000 | 160,000 / 160,000 | 2.1 s | 9, constant |

8,000 was capped by the container's 20,000 file descriptors (both ends are
in one process), not by the runtime. The thread count does not grow with
connections, which is the property `rusty_serve` (thread per connection)
lacks. Listeners `listen(SOMAXCONN)` (`platform-linux`). The reactor is one
level-triggered epoll thread; that is a later throughput question, not a
blocker. Limits of the probe: loopback, plain HTTP, no TLS, tiny bodies; it
is a capability check, not a benchmark against tokio.

So the accept loop does not stop the plan. Whether and how stage 2 starts
is the owner's call (6.5). Much of a stage 2 server driver already exists:

- `rusty_http`: sans-IO HTTP/1.1 with a `rusty_tokio` adapter
  (`AsyncTransport`: request heads, `Content-Length` / chunked / streamed
  bodies, chunked writes for SSE) and a second adapter for real `tokio`.
- `rusty_tls`: `AsyncTlsServerStream` via `TlsAcceptor::accept_async`.
- `rusty_hyper`: an HTTP/2-only server on `rusty_tokio` + `rusty_tls` with a
  connection cap and graceful shutdown (the `rusty_tonic` transport).

New code for the driver is the HTTP/1.1 connection loop (keep-alive,
body limits, graceful shutdown) on those pieces.

### 6.3 What this changes in the plan

1. **Every consumer's handlers run on `tokio`.** `rusty_acp` uses `sqlx` and
   `redis`, `rp-server` `reqwest` and `tokio-postgres` (via `rp-router`),
   `rusty_a2a` `reqwest`, `tonic` and `tokio::sync` / `tokio-stream`, and all
   of them `tokio::spawn` / `tokio::time`. Checked: a `tokio::time::sleep`
   inside a `rusty_tokio` task **panics** ("no reactor"); it works only when
   a tokio `Handle` is entered while the future is created. So a
   `rusty_axum` whose handlers can only run on `rusty_tokio` makes each
   migration also a port off `reqwest` / `sqlx` / `tonic`, which is far
   larger than the web layer. **Recommendation:** keep the framework core
   (router, extractors, responses, middleware, SSE) runtime-free and put the
   runtime in the server driver only, one driver on `rusty_tokio` (the
   target) and one on `tokio` (the bridge), the same split `rusty_http`
   already has. Each consumer then drops axum first and tokio later, as
   separate PRs. This needs `tokio` as an optional dependency of
   `rusty_axum`: **owner decision** (see 6.5).
2. **`agentgateway` is not an axum consumer in the sense the plan assumes.**
   Its server is `hyper-util` (h1 + h2 auto, upgrades, its own TLS
   terminator) with tower layers; axum supplies only a body / response type
   and the MCP mount. Dropping axum there is small (swap the body type and
   the MCP mount once #875's replacement exists). Dropping **hyper and
   tower** there needs an h1 + h2 server and a proxy client, which is a
   separate track from `rusty_axum` (it overlaps `rusty_hyper`, which is
   h2-only today). It stays last either way.
3. **Public APIs return `axum::Router`:** `rusty_a2a::server::AgentServer::
   into_router` / `AgentServices::router`, `rusty_acp::server::AcpServer::
   into_router` / `into_shared_router`, `adk_mcp::http::router`. Moving them
   is a public-API change per crate; `rusty_acp`'s docs invite users to add
   tower middleware to the router it returns, which a non-tower router
   ends.
4. **`rk-app` is the proposed pilot** (pending the owner). It is the smallest app consumer
   and touches the core surface (router, `State`, `Query`, `Json`, SSE,
   CORS, in-process testing). It is still on axum 0.7 (`:param` syntax;
   it has no path parameters, so nothing depends on that).

### 6.4 Found along the way (not fixed here)

`remind_me_remote`'s rate limiter reads the caller address from
`ConnectInfo<SocketAddr>` (`src/auth.rs`), but `server.rs` calls
`axum::serve(listener, router)` without `into_make_service_with_connect_info`,
so the extension is never set. The bucket is chosen before any credential
is validated, and only the connector secret maps to `auth:known`
(`remind_me_core::rate_limit::resolve_key`). Every other caller, including
callers with a valid OAuth access token, therefore shares the one bucket
`ip:unknown`: one noisy client throttles all of them. The gate's bypass
paths (health, the configured allow paths and prefixes) are not limited.
Behind a tunnel the peer would be the tunnel anyway, which limits the
impact. Worth its own issue.

### 6.5 Questions for the owner before stage 2

1. Runtime-free core with a `tokio` driver as a migration bridge (6.3.1), or
   `rusty_tokio` only, with each consumer moving off `tokio` in the same
   step as off axum?
2. Does "drop axum repo-wide" include `[dev-dependencies]`? ADR-0002 allows
   a dev-only wire-format oracle. `rusty_mcp_client_native`'s test hosts
   `rmcp`'s own server, which is a tower service, exactly to test against a
   third-party implementation; it cannot move to `rusty_axum` without
   losing that. Suggested: test mocks move when their crate moves; oracle
   tests keep axum as a dev-dependency.
3. For `agentgateway`: drop axum only (small), and treat hyper / tower as a
   separate later track (6.3.2)?

**Answered by the owner, 2026-10-22 (in the project thread for #879):**

1. **`rusty_tokio`.** The owner asked for `rusty_tokio` rather than a
   `tokio` driver, so `rusty_axum` gets no `tokio` dependency. *Proposed,
   not yet decided:* how a consumer still on `tokio` libraries bridges.
   The proposal is a small adapter in the consumer that enters a
   `tokio::runtime::Handle` on every poll of its handler futures, dropped
   when the consumer leaves `tokio`. A throwaway check (exploratory, not
   production evidence) passed 200 of 200 `rusty_tokio` tasks that each
   used tokio sockets, timers, `spawn` and a oneshot channel this way.
   The cost is two runtimes in one process while a consumer is
   mid-migration. This replaces the recommendation in 6.3.1.
   *Decided by the owner, 2026-10-23:* one shared bridge crate,
   `rusty_axum_tokio`, rather than a copy in each consumer. Its
   `TokioBridge` layer enters the handle on every poll of the handler
   future and of a streamed body (SSE is polled after the handler
   returns). It is the only crate in the stack that names `tokio`, and it
   is deleted with its last consumer.
2. **Yes, dev-dependencies count**, and `rmcp`, the wire-format oracle,
   is to be replaced too, so the suggestion in question 2 is withdrawn.
   ADR-0002 Amendment 1 records the change.
3. **`agentgateway` drops axum only for now.** Hyper is tracked in #885
   and tower in #886.
