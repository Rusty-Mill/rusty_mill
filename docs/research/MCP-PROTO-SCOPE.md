# Scope of `rusty_mcp_proto` (MCP track, step A2)

> **Status:** the scoping record only. The implementation is [#577](https://github.com/Rusty-Mill/rusty_mill/pull/577) (`crates/libs/protocol/rusty_mcp_proto`); a parallel build of the first slices in #586 was dropped in its favour. The "Next" section at the end is superseded.

Method: every `use rmcp...` and inline `rmcp::...` path in the non-Nexus crates (`crates/`, excluding `apps/nexus`) was extracted and flattened to type and macro names. 123 distinct `rmcp` names are used. Counts below are *crates* that use a name. This is a static name inventory; the **wire format has not yet been compared** against `rmcp` (that is the oracle test in A2).

Who uses `rmcp` (names touched): `rusty-mcp` 80, `agentgateway-mcp` 52, `rusty-mcp-demo` 50, `agentgateway` 38, `remind_me_remote` 28, `adk-mcp` 28, `rp-mcp` 27, `rusty_homelab_mcp` 18, `rusty_mcp/template` 17, `rk-app` 14, `rusty-mcp-client` 11.

## What the native crates must provide, in build order

**P0: the minimum every consumer needs (used by 5 or more crates).** `CallToolRequestParams`, `CallToolResult`/`CallToolResponse`, `ContentBlock` (text), `Tool`, `ListToolsResult`, `PaginatedRequestParams`, `ErrorData`/`ErrorCode`, `ServerInfo`/`InitializeResult`, `ServerCapabilities`, `Implementation`, `ProtocolVersion`, `ClientInfo`. Wire methods: `initialize` + `notifications/initialized`, `ping`, `tools/list`, `tools/call`, plus JSON-RPC 2.0 framing (request, response, error, notification, batch handling per the spec revision).

**P1: resources, prompts, completion (3 to 5 crates).** `Resource`, `ResourceTemplate`, `ResourceContents`, `ListResourcesResult`, `ListResourceTemplatesResult`, `ReadResourceRequestParams`/`Result`, `Prompt`, `PromptMessage`, `GetPromptRequestParams`/`Result`, `ListPromptsResult`, `Role`, `Reference`, `ArgumentInfo`, `CompleteRequestParams`/`Result`, `CompletionInfo`/`CompletionContext`. Wire: `resources/list|read|templates/list`, `prompts/list|get`, `completion/complete`.

**P2: the 2026-07-28 additions (1 to 2 crates, all in `rusty-mcp` and `agentgateway`):** cache hints (`CacheScope`, `ttlMs`), tasks (`GetTaskParams`, `UpdateTaskParams`, `CancelTaskParams`, `TaskStatus`, `TaskManager`, `CreateTaskResult`), elicitation and input-required results (`ElicitRequestParams`, `ElicitResult`, `ElicitationAction`, `ElicitationSchema`, `InputRequest(s)`, `InputResponses`, `InputRequiredResult`, `RequestState`, `RequestStateCodec`, `SealOptions`), subscriptions (`SubscriptionFilter`, `SubscriptionContext`), `notifications/cancelled` and progress.

**P3: transports.** stdio (server and client; child-process spawn on the client); Streamable HTTP server (`StreamableHttpService`, `StreamableHttpServerConfig`; session managers: `LocalSessionManager` is used by 4 crates, `NeverSessionManager` by 2) and client (`StreamableHttpClientTransport(+Config)`, 7 crates; needs an SSE reader).

**P4: the handler framework, the part `rmcp` gives for free.** `ServerHandler` (10 crates), `ClientHandler`, `RequestContext`, `RoleServer`/`RoleClient`, `Peer`, `RunningService`, `ServiceExt`, `ToolRouter`, `Parameters`/`Json`, and the macros `#[tool]` (5 crates), `#[tool_router]` (5), `#[tool_handler]` (3), `#[prompt]`/`#[prompt_router]` (1 to 2), with `schemars::JsonSchema` (5 crates) for tool input schemas.

## Decisions applied (owner, 2026-10-09)

- Tool schemas come from a **small first-party builder**, not a derive macro. `#[tool]`-style code becomes `server.tool(name, description, schema, handler)` registrations. The macro-using crates (`rusty-mcp-demo`, `rusty_homelab_mcp`, `remind_me_remote`, `rp-mcp`, template) are the migration cost; five crates, none large.
- Servers are **blocking, one thread per connection**, on `rusty_serve` (and stdio). So `ServerHandler` becomes a plain synchronous trait; `RequestContext`/`Peer` shrink to what blocking handlers need (cancellation flag, progress sender).
- Scope is `rmcp` **and** its stack (`axum`, `tokio`, `reqwest`, `clap`, `tracing-subscriber`, `jsonwebtoken`).

## Observations that shape the plan

1. `agentgateway` and `agentgateway-mcp` are the second-largest consumers and already hand-build JSON-RPC in guardrails and federation (per the audit). They are the strongest argument for a public `rusty_mcp_proto` with typed messages, and they migrate last.
2. P2 is used by a single crate family. Build P0 and P1 first; do P2 only when `rusty-mcp` itself moves.
3. The client is async today (`RunningService` in 9 crates). The native client must offer a blocking API first (matches decision 3); async can wrap it on `rusty_tokio` later if a consumer needs it.
4. `ServiceError` and `McpError` appear in three crates each: define one error enum in the proto crate, with `ErrorCode` constants matching JSON-RPC.

## Next (A2 slice 1)

`rusty_mcp_proto` P0 on `rusty_json`: JSON-RPC 2.0 messages and the P0 types with `to_value`/`from_value`, tested by (a) golden JSON from the spec examples and (b) a dev-only differential test that serialises the same values through `rmcp` and compares.
