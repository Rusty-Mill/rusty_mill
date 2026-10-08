# rusty_mcp_client_native

A first-party MCP client core on [`rusty_mcp_proto`](../rusty_mcp_proto), step
A4 of `docs/research/MCP-NATIVE-PLAN.md`. Depends on `rusty_json` and
`rusty_mcp_proto` only. **This is the first slice:** the pure parts and a
blocking client over a child process. The Streamable HTTP transport (on
`rusty_request`) and the async facade are next; until then the consumers that
use `rmcp`'s client (`rusty-mcp-client`, `rp-mcp`, `adk-mcp`, the gateway) are
not moved. Scope and open decisions: `docs/research/MCP-CLIENT-SCOPE.md`.

| Piece | What it is |
| --- | --- |
| `sse::SseParser` | Server-sent events, pure: bytes in, events out. All three line endings (also split across chunks), a BOM, comments, multi-line `data`, chunk cuts anywhere (inside a character too), and `id`/`retry` that persist for `Last-Event-ID` resumption. |
| `ClientSession` | Sans-IO protocol state: request ids and matching, `server/discover` and `initialize`, the stateless per-request `_meta` (revision, client info, capabilities; the caller's own `_meta` such as a progress token is kept), `notifications/cancelled`. |
| `Transport`, `StdioTransport` | JSON-RPC messages with a receive timeout. `StdioTransport` is newline-delimited JSON over any reader and writer, or a spawned child process (stdin closed, then a 2 s grace, then killed, on drop). Lines that are not messages, and lines over 8 MiB, are skipped. |
| `Client` | A blocking client: the handshake, `call` / `notify` / `ping`, typed `list_tools` (all pages), `list_prompts`, `list_resources`, `list_resource_templates`, `get_prompt`, `read_resource`, `complete`, and `call_tool`, which drives multi-round-trip input and tasks to a result. A `Handler` receives notifications and server requests and answers elicitations. |

## Behaviour worth knowing

- **Handshake.** With a stateless revision configured (the default config
  prefers 2026-07-28), `server/discover` goes first. If the server answers
  method-not-found or unsupported-version, or lists only classic revisions, the
  client falls back to `initialize` (+ `notifications/initialized`). No shared
  revision is `NoCommonVersion`.
- **One call at a time.** `Client` is `&mut`; a call waits for its answer
  while handling whatever else arrives (notifications, server requests).
  Concurrency is the async facade's job.
- **Timeouts.** A call that times out sends `notifications/cancelled` and
  forgets its id, so a late answer is ignored rather than mistaken for the next.
- **`call_tool`.** `input_required` is answered through `Handler::elicit`
  (default: decline) and retried with `inputResponses` and the echoed
  `requestState`, at most 8 rounds. A task is polled with `tasks/get`
  (interval from the server, 10 ms to 1 s) and its questions answered with
  `tasks/update`; a failed or cancelled task is `TaskEnded`. Declare the tasks
  extension and `elicitation` in `ClientConfig::capabilities` or the server will
  not use them. Sampling and roots requests are declined.
- **Server requests** go to `Handler::request`, which refuses by default.

## Tested

18 unit tests (parser, session), 13 against `rusty_mcp_server` in one process
over pipes (both handshakes, fallback, errors, pagination, prompts, resources,
progress, timeout, elicitation, tasks, a vanished server) and 4 against it as a
real child process. Mutation checks (no `_meta`, state not echoed) fail the
intended tests. Not tested: any server other than `rusty_mcp_server`.
