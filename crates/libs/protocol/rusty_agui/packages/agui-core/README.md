# @rusty-mill/agui-core

The headless AG-UI core in TypeScript: the mirror of the `rusty_agui`
crate, with no runtime dependencies. Every framework binding (React first,
then others) sits on this; nothing here knows about a UI.

| Module | What it gives you |
|---|---|
| `types` | The wire types as the TypeScript SDK names them, and `parseEvent(raw)`: known `type`, required members, unknown members kept |
| `jsonPatch` | RFC 6901 pointers, RFC 6902 `applyPatch` (atomic), RFC 7386 `mergePatch` |
| `sse` | `encode(event)`; `Decoder.feed(text)` → events, incrementally; `finish()` |
| `verify` | `Verifier.push(event)` → canonical events: the ordering rules, chunk expansion |
| `reduce` | `Reducer { messages, state }`: fold canonical events into a thread and shared state |
| `run` | `streamAgent(endpoint, input)` → async iterable of verified events; `runAgent(...)` → `{ events, messages, state, result, outcome, error }` |

```ts
import { runAgent } from "@rusty-mill/agui-core";

const result = await runAgent(
  { url: "http://127.0.0.1:8080/api/agent", headers: { Authorization: "Bearer t" } },
  { threadId: "thread-1", runId: "run-1", messages: [{ id: "u1", role: "user", content: "hello" }] },
  { onEvent: (event, view) => render(view.messages, view.state) },
);
// result.messages, result.state; a RUN_ERROR is result.error, not a throw
```

`runAgent` uses the global `fetch` and a streaming body; pass `fetch` on
the endpoint to substitute one (tests do). A non-200 is an `HttpError`;
a non-SSE response or a stream that ends before `RUN_FINISHED` or
`RUN_ERROR` is a `TransportError`; an agent that breaks the ordering
rules is a `SequenceError` on the event that broke them.

## Held to the Rust crate

`../../fixtures/` is shared: every event sample round-trips in both
languages, chunk sequences expand to the same canonical events, and whole
runs reduce to the same messages and state. `../../conformance/` then runs
this core and the reference `@ag-ui/client` against the same `echo_agent`
server.

```
npm ci && npm run typecheck && npm test && npm run build
```
