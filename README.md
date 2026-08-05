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

Authorization is out of scope for this scaffold — terminate OAuth at a gateway
in front of the server, or add a `tower` layer around the mounted service.

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
