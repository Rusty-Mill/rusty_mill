# rusty_a2a

A Rust implementation of the [Agent2Agent (A2A) protocol](https://a2a-protocol.org/latest/) — an open standard for interoperable communication between AI agents.

It provides:

- **`rusty_a2a::types`** — the complete A2A data model (`Task`, `Message`, `Part`, `Artifact`, `AgentCard`, security schemes, ...), transliterated field-for-field from the protocol's normative [`a2a.proto`](spec/a2a.proto), with `camelCase` JSON wire encoding exactly as the spec mandates.
- **`rusty_a2a::client`** (feature `client`) — an async client for calling any A2A agent over JSON-RPC: task lifecycle, SSE streaming, push notification configuration, Agent Card discovery.
- **`rusty_a2a::server`** (feature `server`) — an [`axum`](https://docs.rs/axum)-based harness for building an A2A agent: implement one trait (`AgentExecutor`) and get task state management, streaming, and Agent Card discovery for free, over the JSON-RPC and HTTP+JSON/REST bindings at once. Add the `grpc` feature to serve the same agent state over gRPC too.
- **`rusty_a2a::signing`** (feature `signing`) — sign and verify an `AgentCard` with a JWS (RFC 7515) over its JSON Canonicalization Scheme representation (RFC 8785), per spec Section 8.4.

## Scope

The A2A spec defines three interoperable protocol bindings: **JSON-RPC 2.0**, **gRPC**, and **HTTP+JSON/REST** — this crate implements all three. The server (feature `server`) serves JSON-RPC and HTTP+JSON/REST from the same `axum::Router`/port; add the `grpc` feature and call `AgentServer::build()` to get an `AgentServices` handle that also serves gRPC (via `tonic`), sharing the same task store and executor across all bindings. The client (feature `client`) only speaks JSON-RPC so far. Per spec Section 5.1, an agent only needs to support the protocols it chooses to support; declaring a single `JSONRPC` interface in your Agent Card is fully compliant.

The canonical `a2a.proto` is vendored at [`spec/a2a.proto`](spec/a2a.proto), along with the `google/api/*.proto` files it imports under `spec/googleapis/`; `build.rs` compiles it via `tonic-prost-build` when the `grpc` feature is enabled (requires a `protoc` binary on `PATH`).

Implemented:

- Full data model: `Task`, `TaskStatus`, `TaskState`, `Message`, `Role`, `Part` (text/bytes/url/data), `Artifact`, streaming events, `AgentCard` and all its nested types (security schemes, OAuth flows, extensions, skills), push notification config.
- All 11 A2A operations, over all three protocol bindings: `SendMessage`, `SendStreamingMessage`, `GetTask`, `ListTasks`, `CancelTask`, `SubscribeToTask`, the four push-notification-config CRUD methods, `GetExtendedAgentCard`.
- Agent Card discovery at `/.well-known/agent-card.json`.
- `A2A-Version` / `A2A-Extensions` service parameters.
- The full A2A error model, mapped to JSON-RPC codes (`-32001`..`-32009`), `google.rpc.Status`-shaped REST error bodies with real HTTP status codes, and gRPC status codes — all three derived from one source of truth (`A2aError::grpc_status_name()`).
- Blocking, non-blocking (`returnImmediately`), and streaming (SSE / gRPC server-streaming) message sends, backed by a cooperative-cancellation task executor model.
- Agent Card JWS signing/verification (ES256 and EdDSA).
- `AgentCard.securitySchemes`/`securityRequirements` enforcement, across all three bindings, via a pluggable `AuthVerifier` you register with `AgentServer::with_auth_verifier` — see the `rusty_a2a::server::auth` module docs for exactly what credential material is extracted for each scheme type, and its fail-closed behavior when requirements are declared without a verifier configured.
- Push notification delivery (spec Section 4.3): a webhook POST of the current `Task`, with the config's `token` and `authentication` applied, fired on every status/artifact update - not just CRUD storage of the config.
- `AgentCard.capabilities.extensions[].required` enforcement, across all three bindings: a request that doesn't declare the extension via `A2A-Extensions` is rejected with `ExtensionSupportRequiredError`.
- `SendMessageConfiguration.historyLength` is applied to the task returned by `SendMessage`, matching `GetTask`/`ListTasks` (it has no effect on `SendStreamingMessage`'s live event stream, which never returns a whole `Task` to truncate).
- `SubscribeToTask` reconnection replay: each task keeps a bounded log of its recent events, so reconnecting mid-stream catches up on what was missed instead of only seeing a point-in-time snapshot. JSON-RPC and REST support the standard SSE `Last-Event-ID` reconnect header for precise resume; gRPC has no equivalent field in the canonical request, so a gRPC resubscribe always replays the whole buffered log.
- Multi-tenant isolation: `TaskStore` scopes every task and push notification config by `tenant` (spec Section 4.2) — a task or config created under one tenant (or no tenant at all) is invisible to, unlistable by, and unmutable through a request naming a different tenant. All three bindings pass the caller's real `tenant` through to the store; REST reads it from the JSON body where an operation has one, or a `?tenant=` query parameter otherwise (`GET`s and the body-less `:cancel`/`:subscribe`/`DELETE` actions).

Not implemented (contributions welcome):

- The client (`rusty_a2a::client`) only speaks JSON-RPC — REST and gRPC clients aren't provided, though the `grpc` feature does expose a raw generated `tonic` client (`rusty_a2a::server::grpc::pb::a2a_service_client`) for anyone who wants one.
- `mtls` security schemes are never satisfied by the built-in credential extraction (verifying a client certificate is a TLS-termination-layer concern); an `AuthVerifier` asked to satisfy an `mtls`-only requirement is simply never called for it.
- REST's `additional_bindings` (serving every route again nested under a `/{tenant}` path prefix) aren't implemented — tenant scoping works (see above), just not via that particular URL shape; use the `tenant` body field or `?tenant=` query parameter instead.
- REST's `:subscribe` action is only wired as `POST /tasks/{id}:subscribe`; a spec-literal `GET /tasks/{id}:subscribe` isn't routed.

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
cargo test --features full                       # everything; needs protoc
cargo test --features client,server              # everything but the gRPC suite
cargo test --test wire_format                    # protocol conformance; no features
cargo clippy --features full --all-targets
```

`tests/wire_format.rs` pins the JSON encoding against the vendored [`spec/a2a.proto`](spec/a2a.proto): field names, enum spellings, `oneof` wrapper keys, base64 for `bytes`, RFC 3339 for timestamps, which fields are omitted when unset, and two whole documents (an Agent Card and a `Task`) spelled out literally. Expectations are written as **proto field names** and camel-cased at assert time, so each list diffs line-for-line against the `message` block it cites rather than being hand-transcribed into camelCase — which is where a typo would hide. It needs no features: the data model is always compiled, and its encoding is what every binding and every peer SDK shares. The suites below drive this crate's client against this crate's server, so a name that is wrong symmetrically on both sides passes all of them; this one catches it.

`tests/integration.rs` spins up a real `AgentServer` on a local port and drives it with a real `A2aClient` and a bare `reqwest::Client`, covering the full task lifecycle, streaming, non-blocking sends, cancellation, push notification config CRUD, and the REST binding's routing/error shape, over both bindings sharing one task store. `tests/grpc_integration.rs` does the same against `AgentServices::serve_grpc` with a real generated `tonic` client. `tests/security_and_push_notifications.rs` covers `AuthVerifier` enforcement (accepted/rejected/misconfigured-fail-closed, across JSON-RPC and REST, plus the `GetExtendedAgentCard` auth gate) and push notification delivery to a real local webhook receiver. `tests/history_length_and_extensions.rs` covers `historyLength` truncation on `SendMessage` and required-extension enforcement across JSON-RPC and REST. `tests/subscribe_replay.rs` and `tests/subscribe_replay_grpc.rs` drive a `Notify`-gated agent to deterministically disconnect and reconnect mid-stream, covering `Last-Event-ID` replay on JSON-RPC/REST, the idle-interrupted-task replay-then-snapshot path, and gRPC's coarser whole-buffer replay. `tests/tenant_isolation.rs` and `tests/tenant_isolation_grpc.rs` drive two differently-tenanted clients against the same server, covering cross-tenant `GetTask`/`ListTasks`/`CancelTask`/push-config isolation, REST's `?tenant=` query parameter, and that omitting `tenant` is a single consistent shared namespace (so a single-tenant deployment's behavior is unchanged).

Only the gRPC paths need a `protoc` binary on `PATH`: each suite's `required-features` names the minimum it actually uses, so the JSON-RPC and REST suites run under `--features client,server` alone. A gate that asks for more than its target needs is invisible — cargo drops the target and the suite "passes" by not existing — so CI asserts each suite resolves under its own feature set.

## License

Apache-2.0.
