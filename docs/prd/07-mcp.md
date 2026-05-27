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

#### `StdioMcpClient`

Spawns an MCP server subprocess and speaks JSON-RPC 2.0 over stdin/stdout.
The standard transport for local MCP servers.

```rust
pub struct StdioMcpClient {
    server_name: String,
    child: tokio::process::Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}
```

Startup sequence:
1. Spawn child: `tokio::process::Command::new(command).args(args)`
2. Send `initialize` request; receive `initialized` response
3. Send `tools/list` request; cache descriptors

#### `SseMcpClient`

Connects to a remote MCP server over HTTP with Server-Sent Events for
server→client messages and HTTP POST for client→server calls.

```rust
pub struct SseMcpClient {
    server_name: String,
    base_url: String,
    auth_token: Option<String>,
    client: reqwest::Client,
}
```

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

Two transports, selected via `RUSTYKEYS_MCP_SERVER_TRANSPORT`:

#### `stdio` (default)

The IDE spawns `rusty-keys --mcp` as a subprocess and speaks JSON-RPC 2.0 over
stdin/stdout. Zero IDE configuration beyond pointing at the binary path.

#### `sse`

An `axum` HTTP server on `RUSTYKEYS_MCP_SERVER_PORT` (default 3001) with SSE
for server→client streaming. Enables remote and shared deployments.

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

- **MCP auth**: today env-var token for SSE transport. OAuth per-server is a
  future seam (MCP spec supports it).
- **Dynamic reconnect**: server crash detection via heartbeat; automatic
  reconnect with backoff.
- **Individual tool exposure**: expose each `ToolRegistry` tool directly as an
  MCP tool — letting the IDE call `bash` or `edit_file` without a full session.
- **MCP resources**: the spec supports `resources/list` and `resources/read`;
  a future seam exposes the evidence journal and memory store as MCP resources.
- **Tool schema validation**: validate args against the server-provided JSON
  schema before dispatch — currently deferred to the server.
