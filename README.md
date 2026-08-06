# rusty_a2a

A Rust implementation of the [Agent2Agent (A2A) protocol](https://a2a-protocol.org/latest/) — an open standard for interoperable communication between AI agents.

It provides:

- **`rusty_a2a::types`** — the complete A2A data model (`Task`, `Message`, `Part`, `Artifact`, `AgentCard`, security schemes, ...), transliterated field-for-field from the protocol's normative [`a2a.proto`](spec/a2a.proto), with `camelCase` JSON wire encoding exactly as the spec mandates.
- **`rusty_a2a::client`** (feature `client`) — an async client for calling any A2A agent: task lifecycle, SSE streaming, push notification configuration, Agent Card discovery.
- **`rusty_a2a::server`** (feature `server`) — an [`axum`](https://docs.rs/axum)-based harness for building an A2A agent: implement one trait (`AgentExecutor`) and get task state management, streaming, and Agent Card discovery for free.

## Scope

The A2A spec defines three interoperable protocol bindings: **JSON-RPC 2.0**, **gRPC**, and **HTTP+JSON/REST**. This crate implements the **JSON-RPC 2.0 binding** — the spec's own description of it, "a simple, HTTP-based interface," is also why it's the natural fit for a dependency-light, pure-Rust implementation with no protobuf toolchain required. Per spec Section 5.1, an agent only needs to support the protocols it chooses to support; declaring a single `JSONRPC` interface in your Agent Card is fully compliant.

The canonical `a2a.proto` is vendored at [`spec/a2a.proto`](spec/a2a.proto) for reference (and for anyone who wants to layer a gRPC binding on top with `tonic-build`, reusing `rusty_a2a::types` for the JSON-facing pieces).

Implemented:

- Full data model: `Task`, `TaskStatus`, `TaskState`, `Message`, `Role`, `Part` (text/bytes/url/data), `Artifact`, streaming events, `AgentCard` and all its nested types (security schemes, OAuth flows, extensions, skills), push notification config.
- All 11 A2A operations: `SendMessage`, `SendStreamingMessage`, `GetTask`, `ListTasks`, `CancelTask`, `SubscribeToTask`, the four push-notification-config CRUD methods, `GetExtendedAgentCard`.
- Agent Card discovery at `/.well-known/agent-card.json`.
- `A2A-Version` / `A2A-Extensions` service parameters.
- The full A2A error model, mapped to JSON-RPC codes (`-32001`..`-32009`) and `google.rpc.ErrorInfo`-style error details.
- Blocking, non-blocking (`returnImmediately`), and streaming (SSE) message sends, backed by a cooperative-cancellation task executor model.

Not implemented (contributions welcome): the gRPC and HTTP+JSON/REST protocol bindings, and Agent Card JWS signing/verification.

## Quick start

```toml
[dependencies]
rusty_a2a = { version = "0.1", features = ["server"] }   # or "client", or "full"
```

### Implement an agent

```rust,no_run
use std::sync::Arc;
use async_trait::async_trait;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, AgentSkill, Message, TaskState};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(
            TaskState::Completed,
            Some(Message::agent_text(format!("you said: {}", ctx.message.text()))),
        );
        Ok(())
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let card = AgentCard::new(
        "Echo Agent",
        "Echoes back whatever you send it.",
        "0.1.0",
        AgentInterface::json_rpc("http://localhost:8080"),
    )
    .with_streaming(true)
    .with_skill(AgentSkill::new("echo", "Echo", "Repeats your message back to you."));

    AgentServer::new(card, Arc::new(EchoAgent))
        .serve(([127, 0, 0, 1], 8080))
        .await
}
```

Run the fuller version of this: `cargo run --example echo_server --features server`.

### Call an agent

```rust,no_run
# async fn run() -> rusty_a2a::client::Result<()> {
use rusty_a2a::client::A2aClient;
use rusty_a2a::types::Message;

let (client, card) = A2aClient::discover("http://localhost:8080").await?;
println!("talking to {}", card.name);

let result = client.send_message(Message::user_text("hello!"), None).await?;
println!("{result:?}");
# Ok(())
# }
```

Run it: `cargo run --example send_message --features client -- "hello there"` (against the `echo_server` example above).

### Streaming

```rust,no_run
# async fn run(client: rusty_a2a::client::A2aClient) -> rusty_a2a::client::Result<()> {
use futures_util::StreamExt;
use rusty_a2a::types::Message;

let mut stream = client.send_streaming_message(Message::user_text("hello!"), None).await?;
while let Some(event) = stream.next().await {
    println!("{:?}", event?);
}
# Ok(())
# }
```

### A richer mock agent

`examples/mock_server.rs` is a more complete mock than `echo_server`: five skills routed by keyword, structured data artifacts, a URL artifact, a chunked/appended artifact (streamed piece by piece), a bare-`Message` reply with no task, a `Rejected` task, and an `AuthRequired` interrupted task.

```sh
cargo run --example mock_server --features server   # listens on :8081

A2A_AGENT_URL=http://127.0.0.1:8081 cargo run --example send_message --features client -- "what's the weather in Kyoto?"
A2A_AGENT_URL=http://127.0.0.1:8081 cargo run --example send_message --features client -- "write me a long report on rust"
```

See the file's header comment for the full list of trigger phrases.

## What an `AgentExecutor` looks like

`AgentExecutor::execute` is called once per inbound message. It gets a `RequestContext` (the message, the task it continues if any, and a `CancellationToken` signaled on `CancelTask`) and an `EventSink` to report progress with:

- `events.message(msg)` — reply directly with a message and create no task at all (for pre-task clarification, spec Section 3.7).
- `events.status(state)` / `status_with_message(state, msg)` — move the task through `Working` and on to a terminal (`Completed`/`Failed`/`Canceled`/`Rejected`) or interrupted (`InputRequired`/`AuthRequired`) state.
- `events.artifact(artifact)` / `artifact_update(artifact, append, last_chunk)` — publish results, optionally as incremental chunks.

The harness handles the rest: creating/updating the `Task` in the `TaskStore`, fanning events out to `SendStreamingMessage`/`SubscribeToTask` subscribers, and answering blocking/non-blocking `SendMessage` calls correctly.

By default tasks are held in an in-memory `TaskStore`; implement the `TaskStore` trait yourself to back it with a real database.

## Testing

```sh
cargo test --features full
cargo clippy --features full --all-targets
```

`tests/integration.rs` spins up a real `AgentServer` on a local port and drives it with a real `A2aClient`, covering the full task lifecycle, streaming, non-blocking sends, cancellation, and push notification config CRUD.

## License

Apache-2.0.
