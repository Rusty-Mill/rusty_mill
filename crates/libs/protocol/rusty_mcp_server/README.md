# rusty_mcp_server

An MCP server core on `rusty_mcp_proto` (ADR-0002 Tier S, first-party
dependencies only): describe a server and its tools once, serve it to any
number of clients. Step A3 of `docs/research/MCP-NATIVE-PLAN.md`.

```rust
use rusty_json::Value;
use rusty_mcp_proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{serve_stdio, Server};
use std::sync::Arc;

let mut schema = Value::object();
schema.insert("type", "object");
let server = Server::builder("demo", "0.1.0")
    .tool(Tool::new("hello", schema), |ctx, call| {
        if ctx.is_cancelled() { /* stop early */ }
        Ok(CallToolResult { content: vec![ContentBlock::text("hello")], ..Default::default() })
    })
    .build()?;
serve_stdio(Arc::new(server))?;
```

| Piece | What it does |
|---|---|
| `Server` / `ServerBuilder` | Immutable description: identity, instructions, protocol revisions, page size, in-flight limit, and what it offers: `.tool(..)`, `.prompt(..)`, `.resource(..)`, `.resource_template(..)`, `.completer(..)`. Duplicate names, URIs or templates, a template that cannot be matched, and zero limits are refused at `build`. |
| `Connection` | One client's conversation, no I/O. `start(message)` answers `initialize`, `server/discover` and `ping` at once, handles `notifications/cancelled`, and hands any other request back as a `Job` to `run` on any thread. |
| `CallContext` | What a tool sees: request id, protocol revision, client info and capabilities, `_meta` (trace context in `meta().extra`), `is_cancelled()`, `progress(..)`. |
| `serve_stdio` / `serve_lines` | Newline-delimited JSON; each request runs on its own thread so a cancellation can reach a running tool; an over-long line is refused and skipped. At end of input running requests get `StdioConfig::drain_timeout` (10 s) to finish, then are cancelled, and a tool that still ignores the flag is abandoned after half a second, so the server can always exit. |

| `http` (feature `http`) | Streamable HTTP on `rusty_serve`: `bind_http(server, addr, HttpConfig, Limits)` or mount `HttpHandler` in your own `rusty_serve` server. One `POST` per message, a connection of its own; classic `initialize` opens a lightweight session for cancellation, 2026-07-28 is stateless. |

Both protocol generations are served from the same handlers: a classic client
sends `initialize` and its revision is remembered per connection; a stateless
client names its revision in every request's `_meta` and needs no handshake.
Stateless `tools/list` answers carry `ttlMs` and `cacheScope`.

Prompts, resources and completion work the same way as tools, and the server
does the generic part for you:

- Lists page with opaque, list-tagged cursors (a prompts cursor is refused by
  `resources/list`) and carry `ttlMs`/`cacheScope` on 2026-07-28.
- `prompts/get` refuses a call missing an argument the prompt declares required.
- `resources/read` tries the exact URI first, then templates in registration
  order, and answers `-32002` with the URI in `data` when nothing matches.
  Templates support `{name}` (one path segment) and `{+name}` (anything, with
  slashes); the handler receives the matched variables, still percent-encoded.
  Any other operator is refused at `build` rather than half-supported.
- `completion/complete` checks the reference names a registered prompt argument
  or template, and sends at most 100 values (`total` and `hasMore` say so).
- A method for a feature the server does not register is `-32601`, and its
  capability is not advertised.

## Change notifications

`subscriptions/listen` (2026-07-28 clients) is built in. Create a
`ChangeBroadcaster`, pass it to `ServerBuilder::notify_changes(&broadcaster,
ChangeKinds)`, keep a clone, and publish from anywhere:

```rust
let changes = ChangeBroadcaster::new();
let server = Server::builder("demo", "1")
    .resource(config, read_config)
    .notify_changes(&changes, ChangeKinds::all_resources())
    .build()?;
// ... later, wherever the change happens:
changes.resource_updated("config://demo");
changes.resources_changed();
```

- `ChangeKinds` is what you announce: it sets `tools.listChanged`,
  `prompts.listChanged`, `resources.listChanged` and `resources.subscribe`. A
  category you did not announce is never sent, and announcing a change for a
  feature the server lacks is a build error.
- A listener asks for categories and resource URIs; the server acknowledges
  what it granted (dropping unannounced categories and URIs `resources/read`
  cannot serve) and then forwards matching events, each tagged with the
  subscription id (the request id).
- A listener ends when the client cancels or hangs up, when the transport closes
  (stdio: at end of input, at once, not after the drain timeout), or when
  `broadcaster.close()` is called, which ends every listener with its final
  result. A listener that falls behind its buffer (64 events) is told
  everything it follows may have changed, rather than losing events silently.
- Each open listener holds a request slot (stdio) or a `rusty_serve`
  connection thread (HTTP). Classic clients cannot listen (and the classic
  `resources/subscribe` is not implemented).

Behaviour worth knowing:

- A tool that fails returns a result with `is_error: Some(true)`; return `Err`
  only for a malformed call. A panicking tool becomes an internal error.
- A cancelled request gets no answer. `progress` is silent unless the caller
  sent a progress token.
- Tool input schemas are advertised as given and **not validated**; the
  handler checks its own arguments.
- More than `max_in_flight` concurrent requests, or a reused in-flight id, are
  answered with an error.

Tested against an independent client: `tests/interop.rs` spawns the example
server as a child process and drives it with the `rmcp` client over real stdio,
in both handshake modes (tools, prompts, resources and templates with
pagination, completion, errors, progress, cancellation; both transports share
one fixture and one scenario). `rmcp` is a dev-dependency
only.

## Streamable HTTP

Enable with `features = ["http"]`. Which revision a request speaks comes from
its `_meta` (2026-07-28), else the `MCP-Protocol-Version` header, else
`2025-03-26`. A classic client that never echoes a session id still works.

- A reply is plain JSON when the tool answers within `sse_after` (250 ms) and
  reports no progress; otherwise it is `text/event-stream` with the progress
  notifications, the final response and a `: ping` every `keep_alive` (15 s).
- 2026-07-28 requests must carry `Mcp-Method` (and `Mcp-Name` for
  `tools/call`, `prompts/get`, `resources/*`, `tasks/*`, base64-wrapped when
  needed) agreeing with the body, and a version header agreeing with `_meta`;
  otherwise `400` with `-32020`. Modern errors map to HTTP statuses (`-32602`
  400, `-32601` 404); classic ones stay `200`.
- `Host` must be loopback unless `allowed_hosts` says otherwise; a request with
  an `Origin` not in `allowed_origins` (default none) is refused `403`, which
  keeps web pages away from a local server. Non-browser clients send no `Origin`.
- **Sessions (classic `initialize` only).** The `initialize` reply carries
  `Mcp-Session-Id`. A client that echoes it can cancel with a
  `notifications/cancelled` POST (matched to the request running under that
  session; ids of other sessions never collide) and end the session with
  `DELETE`, which also cancels what it had running. An unknown, ended or idle
  session is `404`. A session keeps only the negotiated revision and the
  cancel flags of running requests, so any POST may still be served by a
  fresh connection; it does not carry the client's `clientInfo` or
  capabilities between POSTs. `max_sessions` (1024; `0` turns sessions off,
  full answers `503`) and `session_idle` (1 h) bound it. 2026-07-28 is
  stateless and has none.
- A client that hangs up on a streaming reply also cancels the call.
- `GET` is `405`: there is nothing to push.
- Each running call or open stream holds a connection thread of `rusty_serve`;
  size `Limits::max_connections` for the concurrency wanted. No TLS: front it
  with a proxy beyond loopback.

Tested with raw sockets (`tests/http.rs`) and against the `rmcp` HTTP client in
both handshake modes (`tests/http_interop.rs`).

Not here yet: classic `resources/subscribe`, tasks and multi-round-trip input
on the server side (and so `resultType: input_required`), authentication (the existing
`rusty-mcp` has OAuth, limits and telemetry that are not ported), and
sessions or stream resumption.
