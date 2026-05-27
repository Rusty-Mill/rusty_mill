# PRD 07 — MCP (Client + Server)

## Responsibility

The MCP crate has two complementary roles:

- **MCP client**: connect to external MCP servers and register their tools in
  `ToolRegistry` alongside Rusty Keys' built-ins. This is how Rusty Keys absorbs
  arbitrary tool ecosystems without writing Rust for each one.
- **MCP server**: expose `Session::send()` over the MCP protocol so that IDEs
  (VS Code, Cursor, JetBrains) can invoke Rusty Keys as a tool source.

Both roles implement the MCP specification (modelcontextprotocol.io, v1) over
JSON-RPC 2.0.

### Built on the official `rmcp` SDK (ADR-0029)

The `mcp` crate is built on **`rmcp`**, the official `modelcontextprotocol/rust-sdk`
(**MIT, tokio-native, v1.7.x**; ADR-0029). `rmcp` owns the MCP wire: the
JSON-RPC 2.0 framing, the `initialize`/`initialized`/`tools/list`/`tools/call`
state machine, and the stdio + streamable-HTTP/SSE transports — for **both** the
client and server roles. Re-implementing that protocol is undifferentiated work
the harness does not own.

Everything specified below as a transport (`StdioMcpClient`, `SseMcpClient`, and
the `--mcp` server transports) is therefore a **thin adapter over `rmcp`**, not
hand-rolled protocol code. Rusty Keys' value-add sits **above** `rmcp` and is the
clean boundary it owns:

- `mcp__<server>__<tool>` **namespacing** (collision-free, policy-addressable);
- the `McpToolFn` → `ToolFn` adapter and the `"ERROR: MCP call failed: …"`
  `ToolOutcome` mapping;
- `McpPolicy::before_tool` vetting (ADR-0007 still vets before dispatch);
- `ApprovalGate::McpToolFirstUse`;
- the **auth header + TLS pins** (the `Authorization: Bearer <token>` convention
  and the non-loopback-`http://`-with-token rejection), passed *into* `rmcp`'s
  transport config rather than re-implemented.

`rmcp` owns framing/lifecycle; Rusty Keys owns policy/namespacing/approval/auth.
The transport structs below become constructors that hand `rmcp` a transport.

> **License note (load-bearing).** `rmcp` is **MIT** → dependency-safe.
> **opendocswork-mcp**, the Rust MCP server that demonstrated `rmcp` doing both
> roles at sub-millisecond dispatch, is **GPL-3.0**: its modular per-tool layout
> is a **reference only — never copy or vendor** it into permissively-licensed
> Rusty Keys. A **`cargo deny` license gate** enforces this: the dependency tree
> must clear allowed-license policy (MIT/Apache-class) before any MCP crate is
> pinned, so a GPL-3.0 dependency cannot enter the build by accident.

## MCP client

### `McpClient` trait

```rust
pub trait McpClient: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<McpToolDescriptor>>;
    async fn call_tool(&self, name: &str, args: serde_json::Value) -> Result<String>;
    fn server_name(&self) -> &str;
}

pub struct McpToolDescriptor {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,   // JSON Schema for args
}
```

### Transport implementations

> Both transports are **adapters over `rmcp`** (ADR-0029): the `McpClient` trait
> survives as the seam above `rmcp`, but the structs below are constructors that
> hand `rmcp` a transport, not protocol implementations.

#### `StdioMcpClient`

Wraps `rmcp`'s child-process transport: spawns an MCP server subprocess and lets
`rmcp` speak JSON-RPC 2.0 over stdin/stdout. The standard transport for local
MCP servers.

```rust
pub struct StdioMcpClient {
    server_name: String,
    // wraps an rmcp child-process transport / client handle (ADR-0029)
    client: rmcp::Client,
}
```

Startup sequence (driven by `rmcp`):
1. Spawn child via `rmcp`'s child-process transport (`command` + `args`)
2. `rmcp` performs the `initialize` / `initialized` handshake
3. `rmcp` issues `tools/list`; descriptors are cached

#### `SseMcpClient`

Wraps `rmcp`'s streamable-HTTP/SSE transport to connect to a remote MCP server:
Server-Sent Events for server→client messages and HTTP POST for client→server
calls. The auth token and TLS pins (below) are passed *into* `rmcp`'s transport
config, not re-implemented.

```rust
pub struct SseMcpClient {
    server_name: String,
    base_url: String,
    auth_token: Option<String>,
    // wraps an rmcp streamable-HTTP/SSE transport (ADR-0029);
    // bearer header + TLS pin supplied as transport config
    client: rmcp::Client,
}
```

##### Auth-header convention

The token resolved from `auth_token_env` (see the config file below) is sent as
a **`Authorization: Bearer <token>`** header on the SSE `GET` and on every
client→server HTTP `POST`. This is the pinned convention; per-server OAuth
remains a future seam.

**TLS is required for any non-loopback `base_url`.** A `https://` (or loopback
`http://127.0.0.1` / `localhost`) URL is accepted; a non-loopback `http://` URL
with an `auth_token` is rejected at connect time so a bearer token is never sent
in cleartext over the network.

##### Remote-vs-local MCP trust

The transport a server uses is also a **trust signal**, and the distinction
matters more than it first appears:

- **Local (stdio) MCP servers are audited host software.** The user chose to run
  the subprocess; its code is on disk, inspectable, and version-pinnable — trust
  it the way you trust any installed dependency.
- **Remote (SSE) MCP servers are untrusted and *mutable after approval*.** A
  remote server can change its behavior — including its tool *descriptions and
  schemas* — between sessions, so the install-time trust decision **expires**.
  `ApprovalGate::McpToolFirstUse` approval does **not** bind the server's future
  behavior, and re-enumeration on reconnect is **not** re-vetting. Treat a remote
  SSE server like the open internet: **run it against fake / non-sensitive data
  first**. (Likewise, a `localhost` listener is not implicitly trusted just
  because it is local — see [threat-model §6](../architecture/threat-model.md#6-gateway--mcp-authentication).)

See [threat-model §1](../architecture/threat-model.md#1-trust-model) for the
authoritative trust-boundary statement.

##### Heartbeat & reconnect (NOW)

Previously listed only as a seam; the client behavior is sketched here because it
governs whether a long-lived SSE connection survives a transient drop:

- **Heartbeat:** the client treats the SSE stream as live only while frames (or
  SSE comment `:` keep-alives) arrive within an idle window; a stall past that
  window is treated as a dropped connection.
- **Reconnect with backoff:** on drop, the client reconnects with bounded
  exponential backoff + jitter (mirroring the aisdk-client retry policy —
  `RUSTYKEYS_RETRY_BASE_MS` / `RUSTYKEYS_RETRY_MAX`), re-sending the bearer
  header and the last event id where the server supports resume. Repeated
  failure surfaces the same way a crashed stdio server does (warn; tools from
  that server return `ERROR: MCP call failed`; `/mcp reconnect` forces a retry).

The exact idle window and whether reconnect is automatic vs operator-triggered is
finalized in **Phase 12**; the convention above is the contract to build to.

### Config file

Servers declared in `.rustykeys/mcp.toml` (path overridden by
`RUSTYKEYS_MCP_CONFIG`):

```toml
[[servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[[servers]]
name = "harness"
transport = "sse"
url = "https://mcp.harness.io/sse"
auth_token_env = "HARNESS_API_KEY"   # env var holding the token

[[servers]]
name = "sqlite"
transport = "stdio"
command = "uvx"
args = ["mcp-server-sqlite", "--db-path", ".rustykeys/memory.db"]
```

### Tool registration

At `Session::new()`, each configured MCP server is connected, its tools
enumerated, and each tool wrapped in a `McpToolFn` adapter:

```rust
pub struct McpToolFn {
    client: Arc<dyn McpClient>,
    descriptor: McpToolDescriptor,
}

impl ToolFn for McpToolFn {
    async fn call(&self, args: serde_json::Value) -> String {
        match self.client.call_tool(&self.descriptor.name, args).await {
            Ok(result) => result,
            Err(e) => format!("ERROR: MCP call failed: {e}"),
        }
    }
}
```

Tool names are namespaced: `mcp__<server_name>__<tool_name>`.
This prevents collisions and lets policy address tools by server.

### Policy integration

MCP tools go through the same `Policy::before_tool()` path as built-ins.
An `McpPolicy` wraps a set of allowlisted / blocklisted (server, tool) pairs:

```rust
pub struct McpPolicy {
    allowed_servers: Option<Vec<String>>,   // None = all allowed
    blocked_tools: Vec<String>,             // fully-qualified: "mcp__server__tool"
}

impl Policy for McpPolicy {
    fn before_tool(&self, name: &str, _args: &serde_json::Value) -> Result<(), PolicyError> {
        if !name.starts_with("mcp__") { return Ok(()); }
        // server-level check
        // tool-level blocklist
        Ok(())
    }
}
```

`ApprovalGate` triggers `McpToolFirstUse { server }` for the first call to each
MCP server's tools in a session.

### `/mcp` CLI command

```
/mcp              → list connected MCP servers and tool counts
/mcp <server>     → list tools for a specific server with descriptions
/mcp reconnect    → reconnect all servers (after a server crash)
```

### Error handling

| Condition | Behaviour |
|---|---|
| Server fails to start | `Session::new()` logs a warning; server skipped |
| `call_tool` returns error | Returns `"ERROR: MCP call failed: …"` as tool result |
| Server crashes mid-session | `reconnect()` spawns a fresh subprocess |
| Schema validation fails | Args rejected before call; returns `"ERROR: invalid args"` |

## MCP server

### Activation

```bash
rusty-keys --mcp
# or
RUSTYKEYS_MODE=mcp rusty-keys
```

Starts the MCP server instead of the CLI REPL.

### Transports

Two transports, selected via `RUSTYKEYS_MCP_SERVER_TRANSPORT`. Both are
`rmcp`-backed (ADR-0029): the `chat`-over-`Session::send()` exposure registers as
an `rmcp` server tool, and `rmcp` owns the transport — the harness layer is still
not bypassed (`rmcp` is transport only).

#### `stdio` (default)

The IDE spawns `rusty-keys --mcp` as a subprocess; `rmcp` speaks JSON-RPC 2.0 over
stdin/stdout. Zero IDE configuration beyond pointing at the binary path.

#### `sse`

`rmcp`'s streamable-HTTP/SSE server transport (an `axum`-class HTTP server) on
`RUSTYKEYS_MCP_SERVER_PORT` (default 3001) with SSE for server→client streaming.
Enables remote and shared deployments.

**Server-side auth.** The SSE server reuses the gateway's bearer-secret pattern:
a configured token must be presented as `Authorization: Bearer <token>` on
connect (the same `auth_token_env` convention the client sends), and TLS is
required for non-loopback binds. This mirrors `RUSTYKEYS_GATEWAY_SECRET` for the
HTTP gateway (PRD 06) so the two surfaces share one auth model. The `stdio`
transport needs no token (the IDE owns the subprocess); auth applies only to the
network-exposed `sse` transport.

### Wire protocol (MCP v1)

```
initialize     → { protocolVersion, capabilities, serverInfo }
initialized    → (notification, no response)
tools/list     → { tools: [{ name, description, inputSchema }] }
tools/call     → { content: [{ type: "text", text: "…" }] }
```

### Exposed tools

The MCP server exposes a `chat` tool mapping to `Session::send()`:

```json
{
  "name": "chat",
  "description": "Send a message to Rusty Keys and receive a reply.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "message": { "type": "string" },
      "session_id": { "type": "string", "description": "Resume a named session" }
    },
    "required": ["message"]
  }
}
```

Future: expose individual tools (`bash`, `edit_file`, etc.) directly for
fine-grained IDE control — each maps to a `ToolRegistry` tool with the same
schema the model sees.

### Session lifecycle

- Each MCP client connection maps to a persistent `Session` (keyed by
  `session_id`, auto-generated if absent)
- Sessions survive across multiple `chat` calls within a connection
- `Session::shutdown()` called on client disconnect
- In `multi` mode, concurrent connections get independent sessions

### Integration with harness layer

The MCP server is a thin transport over `Session::send()` — the same turn cycle,
post-turn work, verification, and evidence journal apply. A `chat` call from an
IDE produces the same `VerificationReport` and episode package as a CLI turn.
The harness layer is not bypassed.

## Seams

- **MCP auth**: the env-var bearer-token convention for the SSE transport (both
  client and server) is now pinned above (*Auth-header convention* /
  *Server-side auth*). **OAuth per-server** remains the future seam (the MCP spec
  supports it).
- **Dynamic reconnect**: the heartbeat + reconnect-with-backoff *behavior* is now
  sketched above (*Heartbeat & reconnect*, NOW); the exact idle window and
  auto-vs-manual policy is finalized in Phase 12.
- **Tool-return inspection** *(v1 intent / seam)*: an optional hook on
  `McpToolFn::call` results (and `web_fetch`) *before* they enter `history`,
  where a small/fast classifier can inspect MCP tool **return values** for
  prompt-injection / poisoned content. This is the **return-path analog of
  `before_tool`** (which guards the *call* path): the tool result is itself an
  attack surface, so input scanning applied to fetched web pages must apply to
  network-enabled tool results with the same rigor. v1 ships the seam plus the
  existing redaction-on-emit; the classifier is a documented future fill (the
  inspector does not need to be the reasoning model). Cross-ref
  [threat-model — tool-return inspection](../architecture/threat-model.md#tool-return-inspection).

- **Individual tool exposure**: expose each `ToolRegistry` tool directly as an
  MCP tool — letting the IDE call `bash` or `edit_file` without a full session.
- **MCP resources**: the spec supports `resources/list` and `resources/read`;
  a future seam exposes the evidence journal and memory store as MCP resources.
- **Tool schema validation**: validate args against the server-provided JSON
  schema before dispatch — currently deferred to the server.
