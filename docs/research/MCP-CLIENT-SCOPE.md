# MCP client scope (step A4)

Read-only inventory, 2026-10-08. Question: what must a first-party MCP client
do so the remaining consumers can drop `rmcp`, and what is missing under it?

## 1. Why this is next

After `rk-app` and `rusty-mcp-demo`, every remaining `rmcp` consumer needs the
**client** (or a server feature the A3 server does not have). No consumer is
server-only and small. `rusty_homelab_mcp` is server-only but waits on the
tool-schema decision (about 58 `#[tool]` tools).

## 2. What each consumer uses

| Consumer | Lines on `rmcp` | Client side | Server side |
|---|---|---|---|
| `rusty-mcp-client` | 1226 | `serve_client`, child process, Streamable HTTP; list/call tools, prompts, resources | none |
| `rk-mcp` | 591 | uses `rusty-mcp-client` (feature `rmcp`) | none |
| `rp-mcp` (`rusty_provider`) | 762 | child process + Streamable HTTP; `CallToolResponse`, `Peer` | 3 native `#[tool]` tools |
| `adk-mcp` | 1390 | `RunningService`, `Peer`, `ServiceError` | `StreamableHttpService` with `NeverSessionManager` |
| `remind_me_remote` | 1651 | none | `StreamableHttpService` with **`LocalSessionManager`**, `ServerSseMessage` (sessions, server push) |
| `agentgateway`, `agentgateway-mcp` | 9500 | Streamable HTTP and stdio clients; **raw** `Peer::send_request` and `ClientJsonRpcMessage` passthrough; `BoxedSseResponse` | `StreamableHttpService` |
| `rusty_homelab_mcp` | 2451 | none | about 58 `#[tool]` tools |
| `nexus-mcp` | 6513 | **frozen, not touched** | |

## 3. What the client must do

1. JSON-RPC over two transports: a **child process** (stdio, newline JSON) and
   **Streamable HTTP** (POST; reply is JSON or an SSE stream; optional
   `GET` push stream; `DELETE` ends a session).
2. Both handshakes: try `server/discover`, fall back to `initialize` on
   method-not-found or unsupported-version (`rusty_mcp_proto::lifecycle` has
   `falls_back_to_initialize` and `pick_common`); keep `Mcp-Session-Id` when
   the server issues one; send `MCP-Protocol-Version`, and `Mcp-Method` and
   `Mcp-Name` on 2026-07-28.
3. Requests with ids, matched to replies; progress and cancellation
   (`notifications/cancelled`, and hang-up cancels on HTTP).
4. The calls consumers make: `tools/list|call`, `prompts/list|get`,
   `resources/list|read|templates/list`, pagination.
5. Driving **multi-round-trip input** and **tasks** (the `rmcp` client's
   `call_tool` loops on `input_required`; `call_tool_once` does not; tasks
   need `tasks/get|update|cancel` polling).
6. A **passthrough** mode for the gateway: send any `Message` and get the
   raw reply or the raw SSE stream, without typed decoding.
7. Server-initiated requests during a call (elicitation, sampling, roots) are
   answered by a client handler; consumers use only elicitation.

## 4. What exists under it

- `rusty_mcp_proto`: every message and type, both lifecycles, `Wire`.
- `rusty_request`: async HTTP client on `rusty_tokio` (own runtime), with a
  `tokio` feature to run on a real tokio runtime (all consumers are on real
  tokio). TLS is `rusty_tls`. It already has `send_streaming()` returning a
  `StreamingResponse` with `chunk()`, so **an SSE reader is a parser over
  `chunk()`; `rusty_request` needs no change for it.** Redirects, cookies,
  proxies, retries and pooling are there. It has no client certificates and
  no HTTP/2 (`rusty_h2` is separate); neither is needed here.
- Not there: a child-process transport (needs `std::process`; a thread per
  pipe is enough), an SSE parser (`id`, `event`, `data`, `retry`, comments,
  `Last-Event-ID` resume), and request/response correlation.

## 5. Proposed shape (for the owner to accept or change)

Mirror the server: a **sans-IO `ClientSession`** (ids, pending table,
handshake state, MRTR and task helpers) over `rusty_mcp_proto`, then thin
transports:

- `rusty_mcp_client_native` (new crate, Tier S like `rusty_mcp_server`; name
  open): `ClientSession`, `sse` (parser, pure), `stdio` (child process on
  threads), `http` (feature, on `rusty_request` with its `tokio` feature).
- Async API on the outside (consumers are async), blocking inside only where a
  thread per pipe is the simplest correct thing (stdio).
- The existing `rusty-mcp-client` keeps its public API and is re-implemented on
  the new crate in step A5, so `rk-mcp` and `rp-mcp` move with it.

## 6. Decisions for the owner (not made here)

1. **Async or blocking API?** Recommended: async, because every consumer is
   on tokio and `rusty_request` is async; stdio uses threads and exposes the
   same async methods through channels. Alternative: blocking everywhere and
   consumers wrap it in `spawn_blocking`.
2. **New crate or grow `rusty_mcp_server`'s siblings?** Recommended: new crate,
   so a server-only binary pulls no client and `rusty_request`.
3. **`rusty-mcp-client`'s public API.** Resolved 2026-10-09 (owner: "go"): it
   changed in place. `McpClient` now returns `rusty_mcp_proto` types
   (re-exported as `rusty_mcp_client::proto`) and runs on the native client;
   its one consumer, `rk-mcp`, moved in the same commit. `rmcp` and `reqwest`
   are gone from that crate.
4. **Server gaps the clients' peers need. (Built 2026-10-08 at the owner's request: the sessions' `GET` push stream with resumption, and classic `resources/subscribe`. The gateway's raw-stream passthrough is not built.)** `remind_me_remote` needs
   server-side sessions with a `GET` push stream and resumption, which the A3
   HTTP transport answers `405`; `agentgateway` proxies streams. Decide
   whether those stay on `rmcp` until the end (A6 cannot finish without them)
   or get built.
5. **Tool schemas** (builder first vs derive) still gates `rusty_homelab_mcp`
   and the 3 native `rp-mcp` tools. Recommended: builder first, as the plan
   says.

## 7. Not verified

Line counts are `rmcp` mentions per crate, not effort. I did not read the
gateway's passthrough code; its need for raw messages is from the import list.
No client code was written.
