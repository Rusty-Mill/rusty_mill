# rusty_mcp

Transport-level building blocks for MCP servers and gateways, in the `rusty-mcp`
crate. It no longer depends on `rmcp` (ADR-0002, Amendment 1).

| Module | What it is |
|---|---|
| `auth` | OAuth 2.1 resource-server authorization: RFC 9728 Protected Resource Metadata, `WWW-Authenticate` challenges, audience-bound bearer tokens. JWT validation with the `jwt` feature. |
| `trace` | Strict W3C trace-context parsing over MCP `_meta` (SEP-414), a `tracing` span carrying the ids, and writing a context back onto an outbound request. |
| `otel` (feature `otel`) | An OTLP trace and metrics pipeline, and `McpMetrics`, a per-request metrics recorder for any server. |
| `serve` (feature `serve`) | `AuthorizedHandler` and `LimitedHandler`: the auth contract and a concurrency limit in front of a blocking `rusty_serve` handler. |

The MCP server and client are separate crates: `rusty_mcp_server` and
`rusty-mcp-client`. Nothing here depends on a server framework. The `tower`
layers (`RequireAuthLayer`, `LimitsLayer`, `McpMetricsLayer`) were removed in
0.12.0 (#886): their one consumer, `agentgateway`, serves on `rusty_axum`,
which sheds load itself (`rusty_axum::Admission`), and records metrics with
`McpMetrics` around its MCP handler:

```rust,ignore
let timer = metrics.start(method_header, name_header);
let response = mcp_service.call(request).await;
timer.finish(status_of(&response));
```

## History

Before 0.6.0 this crate was also an `rmcp`-based server scaffold (`run`, `serve`,
a CLI and config, resources, tasks, subscriptions, completion, and a
`cargo generate` template). Every consumer moved to `rusty_mcp_server`, and the
scaffold was deleted; it is in git history before that change.
