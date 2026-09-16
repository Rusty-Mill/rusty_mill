# MCP (Model Context Protocol)

`rp-mcp` gives rusty_provider a Model Context Protocol surface, built on
[`rusty_mcp`](https://github.com/baileyrd/rusty_mcp) (spec revision
2026-07-28), in both directions at once:

- **Server** (`crates/mcp/src/native.rs`) — rusty_provider's own routing
  exposed as MCP tools: `chat_completion`, `list_models`, `embeddings`. Any
  MCP client can call these directly, and they go through the exact same
  `Router::dispatch`/`embeddings` path as `/v1/chat/completions`/
  `/v1/embeddings` — fallback chains, caching, free-tier tracking all apply.
- **Gateway** (`crates/mcp/src/gateway.rs`) — other, already-running MCP
  servers connected to and re-exposed through the same endpoint, each
  upstream's tools namespaced `"{upstream}/{tool}"` so names never collide.
  `rusty_mcp` only covers the server side of MCP (its `client` feature is
  dev-dependency-only), so this half talks to `rmcp`'s client API directly.

Both are merged into one `tools/list`/`tools/call` surface
(`crates/mcp/src/server.rs`'s `RustyMcpServer`) — a client sees rusty_provider's
own tools and every proxied upstream tool side by side, not two things to
configure separately.

## Configuring

```toml
[mcp]
enabled = true
path = "/mcp"   # optional, this is the default

[[mcp.upstreams]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[[mcp.upstreams]]
name = "example"
transport = "http"
url = "https://mcp.example.com/mcp"
bearer_token_env = "EXAMPLE_MCP_TOKEN"   # optional
```

`[[mcp.upstreams]]` is optional — `[mcp] enabled = true` alone gives you
just the native `chat_completion`/`list_models`/`embeddings` tools, no
gateway proxying. An upstream that fails to connect at startup (bad
command, unreachable URL, wrong `bearer_token_env`) is logged and simply
absent from the tool list, not a hard failure of the whole server — the
same soft-failure pattern `[jwt]`/`[webhook]`/`[persistence]` already use.
It stays absent until `rp-server` restarts; a startup failure isn't
retried, since there's nothing yet to know a connection *dropped from*.

A connection that drops *after* connecting is different: a background
task per upstream reconnects it with exponential backoff, so a transient
outage (the upstream process crashes and gets supervised back up, a
network blip on an HTTP upstream) recovers on its own. Configurable via
`[mcp]`:

```toml
[mcp]
enabled = true
reconnect_backoff_secs = 1        # optional, this is the default
reconnect_backoff_max_secs = 60   # optional, this is the default
# max_reconnect_attempts = 10     # optional -- unset (default) retries forever
```

The delay doubles after each failed attempt, capped at
`reconnect_backoff_max_secs`. While an upstream is down (mid-backoff or
permanently given up on), its tools are simply absent from `tools/list`
and any `tools/call` naming it gets `GatewayError::UnknownUpstream` --
the same shape of error as a typo'd upstream name, not a distinct "it's
reconnecting" state a client needs to handle specially.

## Auth

The MCP endpoint is mounted **inside** rp-server's existing axum app and
port, guarded by the exact same `server.api_key_env`/`[[clients]]`/`[jwt]`
check every other route already goes through (`routes::mcp_auth`, which
just calls the same `check_auth` `/v1/chat/completions` does). This is a
deliberate choice, not an oversight: `rusty_mcp` brings its own OAuth 2.1
resource-server auth model (`rusty_mcp::auth::AuthConfig`), but that's a
second auth system to reconcile with the one this router already has.
`rusty_mcp`'s own docs note that leaving its `auth` unset is "fine behind a
gateway that already authenticates callers" — which is exactly this
deployment shape, so this integration doesn't use `rusty_mcp`'s auth at
all.

If neither `server.api_key_env`, `[[clients]]`, nor `[jwt]` is configured,
the MCP endpoint is unauthenticated, same as every other route in that
case.

## Transports

**Streamable HTTP** (the default) is mounted at `[mcp].path` on the normal
`rp-server` listener. It uses `LocalSessionManager` rather than the newer,
fully stateless `NeverSessionManager` `rusty_mcp` defaults to:
`NeverSessionManager` only accepts clients using spec 2026-07-28's new
stateless `discover` bootstrap, and that revision is barely a month old —
most MCP clients in the wild today, desktop clients included, still only
speak the older `initialize` handshake. `LocalSessionManager` serves both.

**stdio** — for a desktop client that spawns its MCP server as a
subprocess instead of talking HTTP — is available by setting `MCP_STDIO=1`
when starting `rp-server`. With that env var set, `rp-server` skips its
normal HTTP listener entirely and instead serves the same combined tool set
over stdin/stdout via `rusty_mcp::serve(..., ServerConfig::stdio())`. Point
a desktop client's MCP server config at the `rp-server` binary with
`MCP_STDIO=1` set in its environment and a `CONFIG_PATH` pointing at your
`config.toml`.

## Trying it against a real client

Any MCP client (Claude Desktop, the `rmcp` client library, etc.) works. To
sanity-check with `rmcp`'s own client in a throwaway Rust snippet:

```rust
use rmcp::ServiceExt;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

let transport = StreamableHttpClientTransport::from_config(
    StreamableHttpClientTransportConfig::with_uri("http://127.0.0.1:8080/mcp")
        .auth_header("your-server-api-key"),
);
let client = ().serve(transport).await?;
let tools = client.peer().list_tools(None).await?;
println!("{:?}", tools.tools.iter().map(|t| &t.name).collect::<Vec<_>>());
```

`crates/mcp/tests/handler.rs` and `crates/server/tests/http_endpoints.rs`
(`mcp_endpoint_*` tests) do exactly this in-process, and are the best
reference for the exact call shapes if you're integrating a client.

## Out of scope for now

- MCP prompts/resources — tools only.
- Streaming chat completions as an MCP tool (no natural fit without MRTR
  tasks).
- `rusty_mcp`'s own OAuth 2.1 resource-server auth (superseded by the
  [Auth](#auth) section above).
