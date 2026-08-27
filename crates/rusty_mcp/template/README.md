# {{project-name}}

{{description}}

Built on the [`rusty-mcp`](https://github.com/baileyrd/rusty_mcp) scaffold,
targeting MCP specification 2026-07-28.

## Running it

Over stdio, which is what a desktop client launches:

```bash
cargo run
```

Over Streamable HTTP:

```bash
cargo run -- --transport http --bind 127.0.0.1:8080
```

`--help` lists the rest. Every flag has an environment fallback
(`MCP_TRANSPORT`, `MCP_BIND`, …), and `RUST_LOG` overrides `--log`.

## Wiring it into a client

```json
{
  "mcpServers": {
    "{{project-name}}": {
      "command": "/absolute/path/to/target/release/{{project-name}}"
    }
  }
}
```

## Adding a tool

Add a method in `src/server.rs` inside the `#[tool_router]` block:

```rust
#[tool(description = "What it does, written for the model.")]
pub async fn my_tool(&self, Parameters(args): Parameters<MyArgs>) -> String {
    // ...
}
```

The doc comments on the argument struct become the JSON Schema descriptions
the model reads, so write them for the model rather than for a maintainer. The
`description` is what it uses to decide *whether* to call the tool at all.

Return a plain value when the tool cannot fail, or `Result<_, ErrorData>` when
it can. Reserve `ErrorData` for **protocol** errors — a bad argument, a missing
resource. A failure the model should see and reason about belongs in a
successful result instead, so it can try something else.

## Testing

```bash
cargo test
```

`tests/tools.rs` connects a real client over an in-memory pipe, which is the
same code path the stdio transport takes — so it covers dispatch, schema
generation and serialization, not just the tool function.

Anything involving state shared *between* requests needs an HTTP test rather
than a duplex one: Streamable HTTP builds a fresh handler per request, while a
duplex or stdio connection has exactly one for its lifetime, so a duplex test
cannot see that class of bug at all.

## What else the scaffold gives you

None of this is wired up here — add it when you need it:

- **Resources and prompts**, with a registry for resources and URI templates
- **`completion/complete`**, so clients can suggest argument values
- **Authorization**, an OAuth 2.1 resource-server `tower` layer with JWKS
- **The tasks extension**, for tools that take minutes rather than seconds
- **Change notifications** over `subscriptions/listen`
- **Tracing and metrics**, W3C trace context plus OTLP export
- **Load shedding**, a concurrency limit and request timeout

See the [`rusty-mcp` README](https://github.com/baileyrd/rusty_mcp) for each.
