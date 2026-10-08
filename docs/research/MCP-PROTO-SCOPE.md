# `rusty_mcp_proto`: minimal scope (A2 inventory)

Status: **read-only scoping**, 2026-10-08. Feeds step A2 of `MCP-NATIVE-PLAN.md`. No code changed.

Method: every `rmcp::` path in the non-Nexus crates was extracted (`use` trees expanded, inline paths included), then checked against the `rmcp` 3.1.4 source (the version in `Cargo.lock`) for the exact wire shape. Not verified: nothing here was run against a live server; shapes come from reading `rmcp` source, not from captured traffic. Nexus (`crates/apps/nexus`) is excluded by owner decision.

## 1. Headline findings

1. **Two protocol generations are in use.** `rusty-mcp`, `rusty-mcp-demo` and `rusty-mcp-client` target spec **2026-07-28** (stateless: no `initialize`, no session id, `server/discover`, per-request `_meta`). `adk-mcp` pins **2024-11-05 / 2025-03-26 / 2025-06-18** (classic `initialize` handshake). `remind_me_remote` uses `InitializeResult`, an `EventStore`, and session managers (resumable SSE, pre-2026). `rusty_mcp_proto` therefore needs both handshakes, or A5 must drop the old one for those consumers (owner decision, see section 7).
2. **No consumer needs the full `rmcp` model.** Used: tools, prompts, resources (+ templates), completion, pagination, cancel/progress, list-changed and resource subscriptions, tasks, MRTR (elicitation only). Not used by any non-Nexus crate: sampling (`sampling/createMessage`), roots, `logging/setLevel`, `ListRootsRequest`, URL-mode elicitation outside tests, custom requests/notifications, `Extensions`.
3. **Three consumer styles need different layers:** typed model only (`agentgateway`, `rk`), model + handler trait (`adk-mcp`, `remind_me_remote`, gateway), and **macros + schema generation** (`rusty_homelab_mcp` about 58 `#[tool]`, `rusty-mcp-demo`, `rp-mcp` native tools, `rusty-mcp` tasks). Macros are the largest unknown (section 6).
4. **One raw-wire consumer:** `agentgateway-mcp` sends untyped `ClientRequest`/`ServerResult` through `Peer::send_request` and wraps the HTTP client (`StreamableHttpClient`, `StreamableHttpPostResponse`, `BoxedSseResponse`) to inject headers. It needs a client with a pluggable POST/SSE transport hook, not just typed helpers.

## 2. Per-crate inventory

Legend: **M** model types, **H** handler/service types, **T** transport, **X** macros.

| Crate | Role | M | H | T | X |
|---|---|---|---|---|---|
| `rusty-mcp` | server scaffold | tools, prompts, resources, completion, pagination, tasks, MRTR, subscriptions, `ErrorData`/`ErrorCode`, `ProtocolVersion`, `RequestParamsMeta` (trace context) | `ServerHandler`, `RequestContext`, `RoleServer`, `RunningService`, `SubscriptionContext`, `ToolRouter`, `ToolCallContext`, `PromptContext`, `Parameters`, `TaskManager` | stdio, Streamable HTTP server (`LocalSessionManager`, `NeverSessionManager`, `StreamableHttpServerConfig`) | `tool`, `tool_router`, `tool_handler` |
| `rusty-mcp-client` | client (split out) | `CallToolRequestParams`, `CallToolResult`, `Tool`, `Prompt`, `Resource` | `RoleClient`, `RunningService`, `serve_client` | Streamable HTTP client, `TokioChildProcess` (stdio spawn) | none |
| `rusty-mcp-demo` | demo + acceptance server | as `rusty-mcp` plus `ClientCapabilities`, `ElicitRequestParams/Result`, `TaskStatus`, `ServerNotification`, `Role` | `ServerHandler`, `ClientHandler`, `ClientServiceExt`, `ClientLifecycleMode`, `PromptRouter` | HTTP client for tests | `tool`, `tool_router`, `prompt`, `prompt_router`, `Json<T>` |
| `adk-mcp` | ADK tools as MCP server/client | tools list/call, `Implementation`, `ProtocolVersion` (2024-11-05..2025-06-18), `ErrorCode` | `ServerHandler` incl. `supported_protocol_versions`, `Peer`, `ServerInitializeError`, `ServiceError` | HTTP server (`NeverSessionManager`), stdio, child process | none |
| `remind_me_remote` | remote memory server | tools, prompts, resources, `InitializeResult`, `Implementation` | `ServerHandler`, `RequestContext` | HTTP server with session manager and **`EventStore`** (`EventId`, `StreamId`, `EventStream`, `ServerSseMessage`) | none |
| `rusty_homelab_mcp` | homelab tools | `CallToolRequestParams/Result`, `ClientInfo`, `ProtocolVersion`, `ErrorData` | `ServerHandler`, `ClientHandler` (tests), `RoleClient`, `ServiceExt` | stdio, in-process duplex (tests) | `tool`, `tool_router`, `tool_handler`, `Parameters`, `Json<T>` |
| `rp-mcp` (`rusty_provider/crates/mcp`) | gateway + native server | `Tool`, `JsonObject`, `ListToolsResult`, `CallToolResponse`, `ErrorData` | `ServerHandler`, `Peer`, `ServiceError`, `ToolCallContext`, `ToolRouter` | HTTP client, `TokioChildProcess`, `ConfigureCommandExt` | `tool`, `tool_router`, `Parameters`, `Json<T>` |
| `rusty_provider/crates/server` | HTTP host | none (tests use client) | `ServiceExt` | HTTP server (`LocalSessionManager`), HTTP client in tests | none |
| `agentgateway`, `agentgateway-mcp` | MCP gateway/federation | all list/call/get/read results and params, `ResourceContents`, `ServerCapabilities`, `ErrorCode`, `ClientJsonRpcMessage`, `ClientRequest`, `ServerResult`, `GetExtensions` | `ServerHandler`, `RoleClient`, `RunningService`, `Peer::send_request`, `ServiceError` | stdio, HTTP server + client, `TokioChildProcess`, custom `StreamableHttpClient` impl | none |
| `rk-mcp` (`rusty_key/crates/mcp`) | client wrapper | via `rusty-mcp-client` only | via `rusty-mcp-client` | via `rusty-mcp-client` | none |
| `rk-app` (`rusty_key/crates/app`) | stdio server | `CallToolRequestParams`, `CallToolResult`, `Tool`, `ListToolsResult`, `ErrorData` | `ServerHandler`, `RequestContext`, `stdio` | stdio | none |
| `rusty-hister-mcp`, `remind_me_core` | doc comments only | n/a | n/a | n/a | n/a |

`rusty_proxmox` only mentions `rmcp` in a comment.

## 3. Minimal type list for `rusty_mcp_proto`

Everything is plain structs/enums with `camelCase` serde, `skip_serializing_if` on optionals, and `_meta` on most.

**JSON-RPC 2.0 base:** `RequestId` (number or string), `JsonRpcRequest`, `JsonRpcResponse`, `JsonRpcNotification`, `JsonRpcError`/`ErrorData { code, message, data }`, batch not required. `ErrorCode` constants used or produced: `-32700 PARSE_ERROR`, `-32600 INVALID_REQUEST`, `-32601 METHOD_NOT_FOUND`, `-32602 INVALID_PARAMS`, `-32603 INTERNAL_ERROR`, `-32002 RESOURCE_NOT_FOUND`, plus 2026-07-28 `-32022 UNSUPPORTED_PROTOCOL_VERSION`, `-32021 MISSING_REQUIRED_CLIENT_CAPABILITY`, `-32020 HEADER_MISMATCH`.

**Negotiation and metadata:** `ProtocolVersion` (string newtype; constants `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25`, `2026-07-28`), `Implementation { name, title?, version, description?, icons?, websiteUrl? }`, `ServerCapabilities { logging?, completions?, prompts?{listChanged}, resources?{subscribe,listChanged}, tools?{listChanged}, experimental?, extensions? }`, `ClientCapabilities { roots?, sampling?, elicitation?, experimental?, extensions? }` (declare, do not act on roots/sampling), `InitializeRequestParams { protocolVersion, capabilities, clientInfo }`, `InitializeResult { protocolVersion, capabilities, serverInfo, instructions? }`, `DiscoverResult { resultType, supportedVersions, capabilities, instructions?, ttlMs, cacheScope }` (server info rides in `_meta["io.modelcontextprotocol/serverInfo"]`), `MetaObject` (free-form map; carries `traceparent`/`tracestate`/`baggage`), `RequestMetaObject` (`progressToken`, `io.modelcontextprotocol/protocolVersion`, `.../clientInfo`, `.../clientCapabilities`, `.../logLevel`), `ResultType` (string newtype: `complete`, `input_required`, `task`; keep it open, not a closed enum), `CacheScope` (`public`/`private`).

**Pagination:** `PaginatedRequestParams { _meta?, cursor? }`; every list result has `nextCursor?` and, on 2026-07-28, `ttlMs?` + `cacheScope?`. Cursors are opaque strings (the existing `rusty-mcp::pagination` base64s them; that stays outside the proto crate).

**Tools:** `Tool { name, title?, description?, inputSchema, outputSchema?, annotations?, icons?, _meta? }`, `ToolAnnotations`, `ListToolsResult { tools, nextCursor?, ttlMs?, cacheScope? }`, `CallToolRequestParams { name, arguments?, inputResponses?, requestState?, _meta? }`, `CallToolResult { resultType?, content[], structuredContent?, isError?, _meta? }`, `CallToolResponse = Complete | InputRequired | Task`.

**Content:** `ContentBlock` (tagged `type`: `text`, `image`, `audio`, `resource`, `resource_link`), `Annotations`, `Role` (`user`/`assistant`), `Icon`.

**Prompts:** `Prompt { name, title?, description?, arguments?, icons?, _meta? }`, `PromptArgument { name, title?, description?, required? }`, `PromptMessage { role, content }`, `ListPromptsResult`, `GetPromptRequestParams { name, arguments?, inputResponses?, requestState? }`, `GetPromptResult { resultType?, description?, messages }`, `GetPromptResponse = Complete | InputRequired`.

**Resources:** `Resource { uri, name, title?, description?, mimeType?, size?, annotations?, icons? }`, `ResourceTemplate { uriTemplate, name, ... }`, `ResourceContents` (untagged: text `{uri, mimeType?, text}` | blob `{uri, mimeType?, blob}`), `ListResourcesResult`, `ListResourceTemplatesResult`, `ReadResourceRequestParams { uri, inputResponses?, requestState? }`, `ReadResourceResult { resultType?, ttlMs?, cacheScope?, contents }`, `ReadResourceResponse = Complete | InputRequired`.

**Completion:** `Reference` (tagged: `ref/prompt {name}`, `ref/resource {uri}`), `ArgumentInfo { name, value }`, `CompletionContext { arguments? }`, `CompleteRequestParams { ref, argument, context? }`, `CompletionInfo { values, total?, hasMore? }`, `CompleteResult { completion }`.

**Cancel and progress:** `CancelledNotificationParam { requestId?, reason? }`, `ProgressNotificationParam { progressToken, progress, total?, message? }`, `ProgressToken` (number or string).

**List-changed and subscriptions:** `SubscriptionFilter { toolsListChanged?, promptsListChanged?, resourcesListChanged?, resourceSubscriptions? }`, `SubscriptionsListenRequestParams { _meta (requires subscriptionId), notifications }`, notifications `notifications/tools/list_changed`, `.../prompts/list_changed`, `.../resources/list_changed`, `.../resources/updated`. For pre-2026 consumers: `resources/subscribe`, `resources/unsubscribe`.

**Tasks (extension `io.modelcontextprotocol/tasks`):** `CreateTaskResult`, `GetTaskParams { taskId }`, `GetTaskResult`, `UpdateTaskParams { taskId, inputResponses }`, `CancelTaskParams { taskId }`, `TaskStatus`, notification `notifications/tasks`.

**MRTR (stateless multi-round-trip):** `InputRequiredResult { resultType, inputRequests?, requestState? }`, `InputRequests`/`InputResponses` (maps keyed by request id), `InputRequest` (only elicitation is used outside tests), `ElicitRequestParams` (`form {message, requestedSchema}`; `url` variant only if a consumer needs it), `ElicitResult { action: accept|decline|cancel, content? }`, `ElicitationSchema` (a restricted JSON-Schema subset; **start as `serde_json::Value`/`rusty_json::Value`**, the typed 2.4k-line version is not needed). `requestState` signing (`RequestStateCodec`, `SealOptions`) is used by `rusty-mcp::mrtr`; it is crypto, not wire, so it belongs outside `rusty_mcp_proto`.

## 4. Wire messages needed

| Group | Client to server | Server to client | Used by |
|---|---|---|---|
| Handshake (2026-07-28) | `server/discover` (no params; versions and `clientInfo`/`clientCapabilities` ride in `_meta` of every request) | `DiscoverResult` | `rusty-mcp`, demo, client |
| Handshake (classic) | `initialize`, `notifications/initialized`, `ping` | `InitializeResult` | `adk-mcp`, `remind_me_remote` |
| Tools | `tools/list`, `tools/call` | `notifications/tools/list_changed` | all servers, gateway |
| Prompts | `prompts/list`, `prompts/get` | `notifications/prompts/list_changed` | gateway, demo, `remind_me_remote` |
| Resources | `resources/list`, `resources/templates/list`, `resources/read`, (`resources/subscribe`, `resources/unsubscribe` classic) | `notifications/resources/list_changed`, `notifications/resources/updated` | gateway, demo, `rusty-mcp` |
| Completion | `completion/complete` | none | `rusty-mcp`, demo |
| Pagination | `cursor` on every `*/list` | `nextCursor` | all list users |
| Cancel | `notifications/cancelled` (both directions) | same | `rusty-mcp`, client |
| Progress | `_meta.progressToken` on requests | `notifications/progress` | `rusty-mcp`, demo |
| Subscriptions (2026) | `subscriptions/listen` (long-lived SSE response) | list-changed and updated notifications on that stream | `rusty-mcp`, demo |
| Tasks (2026) | `tasks/get`, `tasks/update`, `tasks/cancel` | `notifications/tasks`, `CreateTaskResult` from `tools/call` | `rusty-mcp::tasks`, demo |
| MRTR (2026) | retry with `inputResponses` + `requestState` | `InputRequiredResult` | `rusty-mcp::mrtr`, demo |

**Not needed:** `sampling/createMessage`, `roots/list`, `notifications/roots/list_changed`, `logging/setLevel`, `notifications/message`, `elicitation/create` as a standalone request (only through MRTR/tasks), custom request/notification passthrough.

## 5. Transport behaviour the proto crate must pin down (types only; I/O lives in A3/A4)

- **stdio:** newline-delimited JSON-RPC, one message per line, stderr for logs. `adk-mcp` already caps line length (`line_cap.rs`); `rusty-mcp` has `limits.rs`. Both limits stay in the server layer.
- **Streamable HTTP:** `POST` with JSON-RPC body; response is `application/json` or `text/event-stream`; `MCP-Protocol-Version` header; 2026 mismatch error `HEADER_MISMATCH`. Classic mode adds `Mcp-Session-Id` and `GET` for server push; only `remind_me_remote` (EventStore, `Last-Event-ID`) and the `LocalSessionManager` users need it.
- **SSE framing:** `id`, `event`, `data`, `retry`. A4 needs a reader (already a known gap in the plan); A3 needs a writer.

## 6. Things that are NOT wire types but block `rmcp` removal

| Item | Where used | Size of the problem |
|---|---|---|
| `#[tool]`/`#[tool_router]`/`#[tool_handler]` macros + `schemars` | homelab (about 58 tools), demo, `rp-mcp` native (3), `rusty-mcp` (`tasks.rs`, tests) | Largest. Plan already says builder first; each tool needs `inputSchema` as JSON Schema from a typed struct. Builder = hand-written schema per tool. |
| `ToolRouter`, `PromptRouter`, `Parameters<T>`, `Json<T>` | same | A small registry (name to handler) in a non-proto crate; can be about 150 lines. |
| `RunningService`/`Peer`/`serve_client`/`ServiceExt` | client, gateway, `rp-mcp`, `adk-mcp`, tests | Request/response correlation, cancel, notifications. Belongs in a client/session crate, not the proto crate. |
| `TaskManager`, `TaskContext`, `TaskExit`, `TaskOptions` | `rusty-mcp::tasks`, demo | Server-side runtime, A3. |
| `RequestStateCodec`, `SealOptions` (feature `request-state`) | `rusty-mcp::mrtr` | Sealing needs AEAD/HMAC; check `rusty_oauth`/`rusty_crypto` coverage. |
| `StreamableHttpClient` trait hook | `agentgateway-mcp::mutating_client` | The A4 client must allow injecting per-request headers; design the hook up front. |
| `EventStore` | `remind_me_remote` | Only matters if resumable classic sessions stay. |

## 7. Recommended crate boundary and open questions

Boundary: `rusty_mcp_proto` (Tier S, on `rusty_json`) holds only sections 3 and the method constants of section 4, plus version negotiation helpers. Sessions, routers, tasks, SSE, transports go in separate crates (A3/A4). This keeps it small and testable against `rmcp` as a dev-only oracle: serialize each type with both and compare JSON for fixed fixtures.

Questions for the owner (each changes what A2 builds):

1. **Classic handshake: decided, support both (owner, 2026-10-08).** The proto crate carries `initialize`/`InitializeResult` and `server/discover`/`DiscoverResult`. Server: mode chosen per request (`initialize` or `Mcp-Session-Id` selects classic; `server/discover` or `_meta` protocol version selects stateless), same handlers for both. Client: try stateless, fall back to `initialize` on `-32601` / `-32022`, remember the result per connection. Session ids, push `GET` and `EventStore` live behind a server-crate feature. Build order: stateless first, classic second; `adk-mcp` and `remind_me_remote` stay on `rmcp` until classic lands (they move last in A5).
2. **Task and MRTR types:** include now (needed by `rusty-mcp` and demo) or defer until their servers move in A3? They are about a fifth of the type list.
3. **`ElicitationSchema`:** an opaque JSON value is enough for every current consumer; confirm that is acceptable.

Suggested first code step once answered: `ContentBlock`, `Tool`, `CallToolParams/Result`, list and pagination types, JSON-RPC envelope, `ErrorData` (covers `rk-app` and the gateway's list/call path), with fixtures diffed against `rmcp` 3.1.4.
