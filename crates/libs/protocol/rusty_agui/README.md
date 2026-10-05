# rusty_agui

A sovereign implementation of [AG-UI](https://docs.ag-ui.com/), the
Agent-User Interaction protocol: the event stream between an agent and a
user interface that CopilotKit's React SDK and OpenBot consume. First-party
dependencies only (`rusty_json`, `rusty_json_patch`, optionally
`rusty_serve`); no serde, no async runtime. Root ADR-0007.

## Shape

| Module | What it gives you |
|---|---|
| `types` | `RunAgentInput`, `Message` (a variant per role), `Content`, `Tool`, `Context` |
| `event` | `Event { kind: EventKind, meta }`: all 31 event types, wire names as the TypeScript SDK (`RUN_STARTED`, `messageId`) |
| `codec` | `Event::to_json` / `from_json`, `RunAgentInput::from_json`, and `Value` forms. Unknown members are ignored on decode. |
| `sse` | `encode(&Event)` → `data: {...}\n\n`; `Decoder::feed(bytes)` → events, incrementally |
| `verify` | `Verifier::push(event)` → canonical events: ordering rules enforced, chunk events expanded |
| `reduce` | `Reducer { messages, state }`: fold canonical events into a thread and shared state (deltas via `rusty_json_patch`) |
| `serve` (feature `serve`) | `Agent` trait + `AgentHandler`, a `rusty_serve::Handler` that frames, verifies and streams a run |

## Serve an agent

```rust
use rusty_agui::{Agent, AgentHandler, Emitter, EventKind, RunAgentInput};
use rusty_json::{json, Value};

struct Echo;

impl Agent for Echo {
    fn run(&mut self, input: &RunAgentInput, out: &mut Emitter<'_>) -> rusty_agui::Result<Option<Value>> {
        let last = input.messages.last().map(|m| match m {
            rusty_agui::Message::User { content, .. } => content.text(),
            _ => String::new(),
        }).unwrap_or_default();
        out.state(json!({"turns": 1}))?;                 // STATE_SNAPSHOT
        out.text(&format!("you said: {last}"))?;         // TEXT_MESSAGE_START/CONTENT/END
        out.emit(EventKind::StepStarted { step_name: "done".into() })?;
        out.emit(EventKind::StepFinished { step_name: "done".into() })?;
        Ok(Some(json!({"echoed": last})))               // RUN_FINISHED carries it
    }
}

let server = rusty_serve::Server::bind("127.0.0.1:8080".parse()?, AgentHandler::new(Echo))?
    .with_web_dir("web/dist".into());
server.run()?;
```

`POST` a `RunAgentInput` JSON body and read `text/event-stream` back. The
handler emits `RUN_STARTED` before the agent runs and `RUN_FINISHED` (or
`RUN_ERROR`, with the agent's `Error::Agent` message or the verifier's
rule) after. An event that breaks the ordering rules never reaches the
client: the run ends with `RUN_ERROR` naming the rule.

To mount the agent on one route of an existing `rusty_serve` handler, call
`AgentHandler::handle_run(body)` from your own `Handler`.

Point CopilotKit's React SDK at the endpoint through its runtime
(`HttpAgent({ url })`), or any AG-UI client.

## Consume a stream

```rust
let mut decoder = rusty_agui::sse::Decoder::new();
let mut verifier = rusty_agui::Verifier::new();
let mut view = rusty_agui::Reducer::new();
for event in decoder.feed(&bytes)? {
    for event in verifier.push(event)? {   // expands chunks, checks order
        view.apply(&event)?;
    }
}
// view.messages, view.state
```

## Not here (by choice)

A React component kit (use CopilotKit's), a thread store, a WebSocket
transport (AG-UI's standard transport is SSE), CORS (a `rusty_serve`
concern), and adapters from `rusty_adk`, `nexus` and `rusty_key`'s own
event types (each lands in its own family).

```
cargo test -p rusty_agui --all-features
```
