# rusty_mcp

A reusable Rust scaffold for building [Model Context Protocol][mcp] servers
against the **2026-07-28** specification.

It owns the parts every MCP server needs and nobody wants to write twice —
argument parsing, transport selection, stderr logging, graceful shutdown — so a
new server is a handler plus a three-line `main`. Built on [`rmcp`] 3.x.

```rust
#[tokio::main]
async fn main() -> Result<(), rusty_mcp::ServeError> {
    rusty_mcp::run(|| Ok(MyServer::new())).await
}
```

That binary speaks stdio by default and Streamable HTTP with `--transport http`.

## Why 2026-07-28 matters here

That revision made MCP **stateless**. The `initialize` handshake is gone, along
with `Mcp-Session-Id`, the standalone GET stream, and `Last-Event-ID` stream
resumption. Every request instead carries its own protocol version and client
capabilities in `_meta`.

The practical payoff: an HTTP server built on this scaffold runs behind a plain
round-robin load balancer with **no session affinity and no shared session
store**. Scale it by adding instances.

Two sharp edges this scaffold handles for you:

- **`rmcp`'s `ProtocolVersion::LATEST` still points at `2025-11-25`.** A server
  using `ServerInfo::new` alone advertises the *older* revision even though it
  can speak 2026-07-28. [`rusty_mcp::server_info`] pins the new one. Clients on
  the older revision still negotiate down — there is a test for that.
- **`NeverSessionManager` is the default**, not `LocalSessionManager`, so the
  server refuses to mint sessions at all unless you opt into
  `--legacy-sessions` for pre-2026-07-28 clients.

## Layout

| Path | What it is |
| --- | --- |
| `crates/rusty-mcp` | The reusable scaffold. Depend on this. |
| `crates/rusty-mcp-demo` | An example server built on it. Copy this to start. |

Inside the scaffold:

| Module | Responsibility |
| --- | --- |
| `cli` | `--transport`, `--bind`, `--path`, allow-lists, log filter (with env fallbacks) |
| `config` | `ServerConfig` / `HttpConfig`, if you build config yourself |
| `runtime` | `serve()` — wires a handler to stdio or Streamable HTTP |
| `telemetry` | Logging to **stderr**, never stdout |
| `shutdown` | SIGINT / SIGTERM handling |
| `error` | `ServeError`, plus `ToolError` for tool bodies |
| `auth` | OAuth 2.1 resource-server layer (see [Authorization](#authorization)) |
| `tasks` | Tasks extension for long-running tools (see [Long-running tools](#long-running-tools)) |

## Writing tools

Group tools by topic, one module each. Every module contributes a router; the
server merges them with `+`, so adding a topic never means editing a central
registry.

```rust
#[tool_router(router = calculator_tools, vis = "pub(crate)")]
impl DemoServer {
    #[tool(description = "Divide two integers, returning quotient and remainder.")]
    pub async fn divide(
        &self,
        Parameters(BinaryOp { a, b }): Parameters<BinaryOp>,
    ) -> Result<Json<DivideResult>, ErrorData> {
        if b == 0 {
            return Err(ToolError::invalid("cannot divide by zero").into());
        }
        Ok(Json(DivideResult { quotient: a / b, remainder: a % b }))
    }
}
```

```rust
// crates/rusty-mcp-demo/src/server.rs
tool_router: Self::calculator_tools() + Self::text_tools(),
```

Notes that save time:

- **Return `Result<T, ErrorData>`**, not your own error type — `rmcp` requires
  it. `ToolError` is a shorthand that converts through `?` and `.into()`.
- **Doc comments on argument structs become the JSON Schema descriptions** the
  model reads. They are part of the interface, not decoration.
- **Prefer `Json<T>` for anything structured.** It fills `structuredContent`
  alongside the text block, so clients can consume data without reparsing.
- **A protocol error is not a tool failure.** `ErrorData` means the call could
  not be processed. A tool that ran but failed in its domain ("no such user")
  should return a normal result with `isError: true`, so the model can react.

## Shared state

Streamable HTTP builds a **fresh handler per request**, so anything that must
outlive a single call goes behind an `Arc` captured by the factory closure:

```rust
let state = Arc::new(DemoState::default());
rusty_mcp::serve(move || Ok(DemoServer::with_state(Arc::clone(&state))), config).await
```

## Running it

```bash
# stdio — what Claude Desktop and Claude Code launch locally
cargo run -p rusty-mcp-demo

# Streamable HTTP
cargo run -p rusty-mcp-demo -- --transport http --bind 127.0.0.1:8080
```

Every flag has an environment fallback (`MCP_TRANSPORT`, `MCP_BIND`,
`MCP_PATH`, `MCP_ALLOWED_HOSTS`, …). `RUST_LOG` overrides `--log`.

### Talking to it directly

A 2026-07-28 request carries `_meta` *and* the matching `MCP-Protocol-Version`
header — the transport rejects a mismatch with `-32020`:

```bash
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2026-07-28' \
  -H 'Mcp-Method: tools/list' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{}}}}'
```

### Wiring into a client

```json
{
  "mcpServers": {
    "rusty-mcp-demo": {
      "command": "/path/to/target/release/rusty-mcp-demo"
    }
  }
}
```

## Deploying over HTTP

The `Host` allow-list defaults to **loopback only**, which guards local servers
against DNS rebinding. A public deployment must set its own hostnames, and any
browser-reachable server should set origins too:

```bash
rusty-mcp-demo --transport http --bind 0.0.0.0:8080 \
  --allowed-host mcp.example.com \
  --allowed-origin https://app.example.com
```

## Long-running tools

A tool that takes minutes should not hold a request open for minutes. The
[tasks extension][tasks] (`io.modelcontextprotocol/tasks`, SEP-2663) lets the
server answer `tools/call` with a **task handle**; the client polls `tasks/get`
until it settles. In 2026-07-28 this moved out of the core protocol into an
opt-in extension, and the blocking `tasks/result` became polling.

Add a `TaskSupport` to your server, advertise the capability, and forward the
three task methods:

```rust
use rusty_mcp::tasks::{TaskPolicy, TaskSupport};

#[derive(Clone)]
pub struct MyServer {
    tasks: TaskSupport,
    tool_router: ToolRouter<Self>,
}

// Only name the tools that are actually slow.
let tasks = TaskSupport::with_policy(TaskPolicy::named(["countdown"]));

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MyServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info("my-server", "0.1.0",
            ServerCapabilities::builder().enable_tools().enable_tasks().build())
    }

    rusty_mcp::forward_task_methods!(tasks);
}
```

Then let `TaskSupport::run` decide per call, inside an ordinary `#[tool]` fn:

```rust
#[tool(description = "Count down in steps, slowly.")]
pub async fn countdown(
    &self,
    Parameters(args): Parameters<CountdownArgs>,
    ctx: RequestContext<RoleServer>,
) -> Result<CallToolResponse, ErrorData> {
    self.tasks.run(&ctx, COUNTDOWN, move |task| countdown_body(args, task)).await
}
```

Two things make that work: taking `RequestContext` gives the tool the
per-request client capabilities, and returning `CallToolResponse` is what lets
it hand back a handle instead of a result.

### Why one body, not two

**A task handle may only go to a client that declared the extension.** Send one
to a client that did not and dispatch rejects it with `-32021`, which surfaces
as a confusing failure rather than graceful degradation. So every task-capable
tool has to work both ways.

`run` picks the path and executes the same body either way. Write it against
`TaskCtx`, which degrades to no-ops off-task — `set_status_message` does
nothing, and `cancelled()` never resolves, so it stays safe as a `select!` arm:

```rust
tokio::select! {
    _ = ctx.cancelled() => return Err(TaskExit::Cancelled),
    _ = tokio::time::sleep(step) => {}
}
```

Writing the two paths separately is how they drift — a fix lands on one and not
the other.

### Notes

- **Clone one `TaskSupport`, don't construct per request.** Streamable HTTP
  builds a fresh handler per request while tasks outlive the call that created
  them; a new manager per request means every poll misses.
- **Cancellation is cooperative.** `tasks/cancel` is acknowledged immediately
  and the task settles as `cancelled` only if its body checks. A body that
  ignores `cancelled()` runs to completion regardless.
- **TTL bounds memory**, defaulting to an hour. Too short and a slow-polling
  client loses the result it was waiting for.

[tasks]: https://modelcontextprotocol.io/seps/2663-tasks-extension

## Authorization

Set `HttpConfig::auth` and the endpoint becomes an OAuth 2.1 **resource
server**: `RequireAuthLayer` guards the MCP route, and the RFC 9728 Protected
Resource Metadata document is published *unauthenticated* alongside it.

```rust
use rusty_mcp::auth::{AuthConfig, StaticTokenValidator, VerifiedToken};

let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(my_validator))?
    .with_authorization_servers(["https://auth.example.com"])
    .with_scopes_supported(["mcp:read"])
    .with_required_scopes(["mcp:read"]);

let config = ServerConfig {
    transport: Transport::Http(HttpConfig { auth: Some(Arc::new(auth)), ..Default::default() }),
    ..Default::default()
};
```

Bring your own validation by implementing `TokenValidator` — JWT signature
checks, RFC 7662 introspection, a lookup in your own store. `StaticTokenValidator`
is for tests and local development only; it does no cryptography.

### What the layer enforces

| Situation | Status | `WWW-Authenticate` |
| --- | --- | --- |
| No `Authorization` header | 401 | `Bearer scope=…, resource_metadata=…` (no `error`, per RFC 6750 §3.1) |
| Not `Bearer`, or empty token | 400 | `error="invalid_request"` |
| Token fails validation or is expired | 401 | `error="invalid_token"` |
| Token minted for another resource | 401 | `error="invalid_token"` |
| Valid token, missing scopes | 403 | `error="insufficient_scope", scope="…"` |
| Validator itself is down | 503 | *(none)* |

Three of those are worth calling out:

- **Audience binding is enforced by the layer, not left to your validator.**
  The spec's "MCP servers **MUST NOT** accept or transit any other tokens" is
  aimed at a confused deputy — someone replaying a token minted for a different
  service to borrow this server's privileges. A validator that forgets the
  `aud` check would reopen that hole, so `AuthConfig::resource` is checked
  centrally. If your validator provably binds the audience itself, say so with
  `VerifiedToken::audience_checked_by_validator()`.
- **Insufficient scope is 403, not 401.** Re-authenticating doesn't help; the
  client needs a *broader* token. The challenge names every missing scope at
  once, because drip-feeding them forces one authorization round trip per scope.
- **A validator outage is 503 with no challenge.** A 401 would tell the client
  its perfectly good token is bad and send the user through a pointless login.

### The metadata URL is not where you'd guess

RFC 9728 §3.1 inserts the well-known segment *before* the resource path. For
resource `https://mcp.example.com/mcp` the document lives at:

```
https://mcp.example.com/.well-known/oauth-protected-resource/mcp
```

`AuthConfig::metadata_url()` and `metadata_path()` derive this for you.

### Per-tool scopes

`required_scopes` guards the whole endpoint. For finer grain, leave it empty and
check inside a tool — the layer puts the `VerifiedToken` in the request
extensions, which the transport forwards as `http::request::Parts`:

```rust
let token = ctx
    .extensions
    .get::<http::request::Parts>()
    .and_then(|parts| parts.extensions.get::<VerifiedToken>());
```

Scope checks are plain set containment. If your scheme has hierarchies where a
broader scope implies narrower ones, expand the implied scopes in your validator
so they land in `VerifiedToken::scopes`.

### Not included

Only the resource-server half is here. Running an authorization server, the
client-side flow (PKCE, `resource` parameter, RFC 9207 `iss` validation), and
token issuance are all separate concerns — front this with a real
authorization server such as Keycloak, Auth0, or WorkOS.

Authorization is HTTP-only by design: the spec says stdio servers **SHOULD NOT**
use it and should read credentials from the environment instead.

## Development

```bash
cargo test --workspace     # unit, integration (both transports), and doc tests
cargo clippy --workspace --all-targets
cargo fmt --all
```

The HTTP tests run over a real TCP socket with the `rmcp` client, so they cover
the whole path: `serve` → axum → `StreamableHttpService` → handler.

## Notes on the client side

`rmcp`'s default client still uses the legacy `initialize` handshake. To get a
stateless 2026-07-28 client, ask for it explicitly:

```rust
ClientInfo::default()
    .serve_with_lifecycle(
        transport,
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await?
```

## License

Apache-2.0. See [LICENSE](LICENSE).

[mcp]: https://modelcontextprotocol.io/specification/2026-07-28
[`rmcp`]: https://crates.io/crates/rmcp
[`rusty_mcp::server_info`]: crates/rusty-mcp/src/lib.rs
