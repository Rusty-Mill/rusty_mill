# rusty_mcp_client_native

A first-party MCP client core on [`rusty_mcp_proto`](../rusty_mcp_proto), step
A4 of `docs/research/MCP-NATIVE-PLAN.md`. Depends on `rusty_json` and
`rusty_mcp_proto` only (HTTP, behind the `http` feature, adds `rusty_request`,
`rusty_tokio` and `rusty_base64`). **Second slice:** the pure parts, a blocking
client over a child process or Streamable HTTP. The async facade is next; until
then the consumers that use `rmcp`'s client (`rusty-mcp-client`, `rp-mcp`,
`adk-mcp`, the gateway) are not moved. Scope and open decisions: `docs/research/MCP-CLIENT-SCOPE.md`.

| Piece | What it is |
| --- | --- |
| `sse::SseParser` | Server-sent events, pure: bytes in, events out. All three line endings (also split across chunks), a BOM, comments, multi-line `data`, chunk cuts anywhere (inside a character too), and `id`/`retry` that persist for `Last-Event-ID` resumption. |
| `ClientSession` | Sans-IO protocol state: request ids and matching, `server/discover` and `initialize`, the stateless per-request `_meta` (revision, client info, capabilities; the caller's own `_meta` such as a progress token is kept), `notifications/cancelled`. |
| `Transport`, `StdioTransport` | JSON-RPC messages with a receive timeout. `StdioTransport` is newline-delimited JSON over any reader and writer, or a spawned child process (stdin closed, then a 2 s grace, then killed, on drop). Lines that are not messages, and lines over 8 MiB, are skipped. |
| `HttpTransport` (feature `http`) | Streamable HTTP on `rusty_request`, on a private `rusty_tokio` runtime so the blocking client works unchanged. See below. |
| `Client` | A blocking client: the handshake, `call` / `notify` / `ping`, typed `list_tools` (all pages), `list_prompts`, `list_resources`, `list_resource_templates`, `get_prompt`, `read_resource`, `complete`, and `call_tool`, which drives multi-round-trip input and tasks to a result. A `Handler` receives notifications and server requests and answers elicitations. |

## Streamable HTTP

- **Replies.** `202` is nothing; `application/json` is one message (or a batch);
  `text/event-stream` is parsed with `SseParser` and each `message` event
  delivered as it arrives (progress notifications first, then the answer).
- **Headers.** After the handshake every request carries `MCP-Protocol-Version`;
  on a stateless revision also `Mcp-Method` and, where the method has one,
  `Mcp-Name` (`=?base64?...?=` when not header-safe). Optional bearer token and
  extra headers in `HttpConfig`.
- **Sessions.** `Mcp-Session-Id` is echoed; dropping the transport sends
  `DELETE` (bounded to 2 s). On a classic session the transport opens the
  standalone `GET` stream itself and reconnects with `Last-Event-ID` (or the
  server's `retry`) until the server says `405`/`404`/`400`/`406` or the
  transport is dropped; use `Client::pump` to receive what arrives between calls.
- **Cancellation.** A timed-out or cancelled request aborts its POST, closing
  the connection (that is how a stateless server learns of it); a classic session
  also gets `notifications/cancelled`.
- **Failures.** A POST that fails below JSON-RPC becomes a JSON-RPC error for its
  request, so a waiting call fails at once; a notification's failed POST is
  counted in `failed_notifications()`. A 4xx to `server/discover` is read as
  method-not-found, so the client falls back to `initialize` (the `rmcp`
  server answers `422`).
- **Not done.** Resuming an interrupted POST event stream (a stream cut before
  its answer leaves the call to time out); `subscriptions/listen` as a
  long-lived call (the blocking client has one call at a time); TLS settings
  beyond `rusty_request`'s defaults (system trust store).

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
over pipes, 4 against it as a child process, and with `http`: 10 against its
HTTP transport over sockets (both revisions, sessions, event-stream replies,
questions and tasks across POSTs, pushed updates, hang-up and classic
cancellation, refused connections, the exact headers) and 2 against **`rmcp`'s own
Streamable HTTP server**, with and without sessions, the one independent peer
so far. Mutation checks (no `_meta`, state not echoed, cancel not aborting the
POST, session id or `Mcp-Name` not sent, no push stream) fail the intended
tests. Not tested: any other server, HTTPS, an authenticating server.
