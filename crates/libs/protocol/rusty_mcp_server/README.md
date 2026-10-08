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

## Asking the user mid-call (multi-round-trip input)

On 2026-07-28 a tool that needs the user's input does not call the client; it
answers `input_required` and the client retries with the answers. Register it
with `interactive_tool` and return `ToolOutcome::Ask`:

```rust,ignore
.interactive_tool(tool, |ctx, call| {
    let Some(reply) = answer(&call, "confirm")? else {
        let form = ElicitParams::Form { message: "Book it?".into(), requested_schema, meta: None };
        return Ok(ToolOutcome::Ask(Ask::new().elicit("confirm", &form).with_state("draft:42")));
    };
    // ... reply.action, reply.content; ctx.request_state() is "draft:42" ...
})
```

- The server refuses an `Ask` (never reaches the client) when the revision is
  not 2026-07-28 (`-32600`), when the client declared no `elicitation`
  capability (`-32021`), or when the ask is empty (`-32603`).
- **`requestState`** (feature `request-state`; pulls in `rusty_oauth`'s
  HMAC-SHA256, `rusty_crypto_key` and `rusty_rand`, all first-party). State
  left with `Ask::with_state` is sealed: HMAC over the bytes, an expiry and
  the method and tool name, so a client cannot alter it, extend it, or use it
  on another tool. A bad, foreign or expired one is `-32602` before the
  handler runs. It is **not encrypted** (the client can read it) and it can be
  replayed until it expires (10 minutes; `state_ttl`), so keep in it only what
  the retry re-checks. Servers answering from several processes must share
  `state_key` (32+ bytes); the default is a random key per build, so a state
  from another process is refused and the client starts over.
- Not bound to a client identity: if requests are authenticated, check the
  caller again on the retry.
- Tools only for now; prompts and resources take no input yet. A task asks
  its own questions (below).

## Tasks

Register a long-running tool with `task_tool`. A call from a client that
declared the `io.modelcontextprotocol/tasks` extension is answered at once with
a task; the handler runs on a thread of its own and the client polls
`tasks/get`, answers its questions with `tasks/update` and may `tasks/cancel`.

```rust,ignore
.task_tool(tool, |task, call| {
    task.set_message("crunching");
    let reply = task.elicit("ok", &form).map_err(|_| cancelled_error())?;
    // poll task.is_cancelled() in long loops
    Ok(result)
})
```

- The server advertises the extension once it has a task tool. A client
  that did not declare it gets `-32021`; classic revisions have no tasks
  (`-32600`). Task tools are task-only: there is no synchronous fallback.
- `TaskContext::elicit` shows the task as `input_required` with the question
  and blocks until `tasks/update` answers it (a malformed or unrelated answer
  is `-32602` and the task keeps waiting), or the task is cancelled.
- A failing or panicking handler fails its task; a handler that returns
  after `tasks/cancel` does not bring the task back. Unknown ids and ending a
  finished task are `-32602`.
- **The store is in memory**, shared by all connections of a `Server`, so
  stateless HTTP clients poll from any POST. It does not survive a restart or
  span instances. `max_tasks` (1000; more is `-32603 too many tasks`) and
  `task_ttl` (1 h from creation, whole seconds) bound it; an expired task is
  cancelled and forgotten.
- A task id is 128 random bits and is the only credential: anyone holding it
  can read, answer and cancel the task. Put authentication in front of the
  transport if ids could leak.
- Status changes are polled; `notifications/tasks` is not sent. Tasks are not
  advertised per tool (no `execution` metadata).

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

Not here yet: classic `resources/subscribe`, `notifications/tasks`, multi-round-trip
input for prompts and resources, authentication (the existing `rusty-mcp` has
OAuth, limits and telemetry that are not ported), TLS, and stream resumption.
