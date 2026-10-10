# rusty_mcp

Transport-level building blocks for MCP servers and gateways, in the `rusty-mcp`
crate. It no longer depends on `rmcp` (ADR-0002, Amendment 1).

| Module | What it is |
|---|---|
| `auth` | OAuth 2.1 resource-server authorization: RFC 9728 Protected Resource Metadata, `WWW-Authenticate` challenges, audience-bound bearer tokens as a `tower` layer (`RequireAuthLayer`). JWT validation with the `jwt` feature. |
| `limits` | `LimitsLayer`: bounded concurrency and a response timeout that shed load with `503` and `Retry-After` instead of queueing. |
| `trace` | Strict W3C trace-context parsing over MCP `_meta` (SEP-414), a `tracing` span carrying the ids, and writing a context back onto an outbound request. |
| `otel` (feature `otel`) | An OTLP trace and metrics pipeline, and `McpMetricsLayer` for per-request metrics. |

The MCP server and client are separate crates: `rusty_mcp_server` (with
`rusty_mcp_axum` to mount it in an axum app) and `rusty-mcp-client`. The layers
here go in front of that mount; the tests in `crates/rusty-mcp/tests` run them
against a real `rusty_mcp_server` that way.

```rust,ignore
let mcp = rusty_mcp_axum::router(handler, 1 << 20)
    .layer(RequireAuthLayer::new(auth))      // inner: authorization
    .layer(LimitsLayer::new().with_max_concurrent(256)); // outer: shed first
```

## History

Before 0.6.0 this crate was also an `rmcp`-based server scaffold (`run`, `serve`,
a CLI and config, resources, tasks, subscriptions, completion, and a
`cargo generate` template). Every consumer moved to `rusty_mcp_server`, and the
scaffold was deleted; it is in git history before that change.
