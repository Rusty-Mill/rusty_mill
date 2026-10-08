# rusty_mcp_server

An MCP server core on `rusty_mcp_proto` (ADR-0002 Tier S, first-party
dependencies only): describe a server and its tools once, serve it to any
number of clients. Step A3 of `docs/research/MCP-NATIVE-PLAN.md`, first slice.

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
| `Server` / `ServerBuilder` | Immutable description: identity, instructions, protocol revisions, tools, page size, in-flight limit. Duplicate tool names and zero limits are refused at `build`. |
| `Connection` | One client's conversation, no I/O. `start(message)` answers `initialize`, `server/discover` and `ping` at once, handles `notifications/cancelled`, and hands any other request back as a `Job` to `run` on any thread. |
| `CallContext` | What a tool sees: request id, protocol revision, client info and capabilities, `_meta` (trace context in `meta().extra`), `is_cancelled()`, `progress(..)`. |
| `serve_stdio` / `serve_lines` | Newline-delimited JSON; each request runs on its own thread so a cancellation can reach a running tool; an over-long line is refused and skipped. At end of input running requests get `StdioConfig::drain_timeout` (10 s) to finish, then are cancelled, and a tool that still ignores the flag is abandoned after half a second, so the server can always exit. |

Both protocol generations are served from the same handlers: a classic client
sends `initialize` and its revision is remembered per connection; a stateless
client names its revision in every request's `_meta` and needs no handshake.
Stateless `tools/list` answers carry `ttlMs` and `cacheScope`.

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
in both handshake modes (list with pagination and cache hints, calls, tool
failures, malformed calls, progress, cancellation). `rmcp` is a dev-dependency
only.

Not here yet: prompts, resources, completion, subscriptions, tasks and
multi-round-trip input on the server side, and the Streamable HTTP transport.
