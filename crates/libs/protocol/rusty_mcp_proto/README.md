# rusty_mcp_proto

MCP wire types on `rusty_json`: the JSON-RPC 2.0 envelope and the Model
Context Protocol's methods and payloads as plain Rust types, with a
hand-written codec and no serde, transport or async. ADR-0002 Tier S.

A2 of `docs/research/MCP-NATIVE-PLAN.md`, in slices. So far:

| Module | Types |
|---|---|
| `rpc` | `Message` (request, notification, response, error), `RequestId`, `ErrorData`, `ErrorCode` |
| `version` | `ProtocolVersion` (classic and stateless 2026-07-28), `Implementation` |
| `content` | `ContentBlock` |
| `resource` | `Resource`, `ResourceContents`, `ResourceTemplate`, list/read params and results |
| `page` | `PaginatedParams`, `Paging` (cursor, `ttlMs`, `cacheScope`), `ResultType` |
| `tool` | `Tool`, `ListToolsResult`, `CallToolParams`, `CallToolResult` |
| `prompt` | `Prompt`, `PromptMessage`, `Role`, `ListPromptsResult`, `GetPromptParams`, `GetPromptResult` |
| `completion` | `Reference`, `CompleteParams`, `CompleteResult` |
| `capabilities` | `ServerCapabilities`, `ClientCapabilities` and the typed `prompts`/`resources`/`tools` sub-capabilities |
| `lifecycle` | `InitializeParams/Result` (classic), `DiscoverParams/Result` (stateless), `negotiate_classic`, `pick_common`, `falls_back_to_initialize` |
| `meta` | `RequestMeta`: typed view of a request `_meta` (progress token, protocol version, client info and capabilities, log level), other keys kept |
| `notify` | `CancelledParams`, `ProgressParams`, `ProgressToken` |

Each module has a `method` submodule with the method-name constants.

```rust
use rusty_mcp_proto::{CallToolParams, Message, RequestId, Wire};

let call = Message::request(RequestId::Number(1), "tools/call", &CallToolParams::new("add"));
assert_eq!(Message::from_json(&call.to_json()).unwrap(), call);
```

Every type implements `Wire` (`to_value`/`from_value`/`to_json`/`from_json`).
Encoded objects list their keys in sorted order (`rusty_json`'s `Map`), which JSON-RPC does not care about. `_meta`, annotations, icons, schemas and error data stay raw
`rusty_json::Value` so a gateway forwards them untouched; unknown members of
known types are dropped on decode.

`tests/oracle.rs` checks each type against `rmcp` 3.1's serde model
(dev-dependency only): the fixture, this crate's re-encoding and `rmcp`'s
re-encoding must agree, and malformed input must be refused by both.

Not yet covered: subscriptions (`subscriptions/listen`, list-changed and
updated notifications), tasks, and multi-round-trip input (`InputRequired`
results, elicitation).
Scope: `docs/research/MCP-PROTO-SCOPE.md`.
