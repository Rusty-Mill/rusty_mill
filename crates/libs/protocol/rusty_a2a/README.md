# rusty_a2a

A Rust implementation of the [Agent2Agent (A2A) protocol](https://a2a-protocol.org/latest/) — an open standard for interoperable communication between AI agents.

It provides:

- **`rusty_a2a::types`** — the complete A2A data model (`Task`, `Message`, `Part`, `Artifact`, `AgentCard`, security schemes, ...), transliterated field-for-field from the protocol's normative [`a2a.proto`](spec/a2a.proto), with `camelCase` JSON wire encoding exactly as the spec mandates.
- **`rusty_a2a::client`** (feature `client`) — an async client for calling any A2A agent: task lifecycle, streaming, push notification configuration, Agent Card discovery. `A2aClient` speaks JSON-RPC; `client::RestClient` speaks HTTP+JSON/REST; `client::GrpcClient` (feature `client` + `grpc`) speaks gRPC — all three expose the same method names and signatures.
- **`rusty_a2a::server`** (feature `server`) — an [`axum`](https://docs.rs/axum)-based harness for building an A2A agent: implement one trait (`AgentExecutor`) and get task state management, streaming, and Agent Card discovery for free, over the JSON-RPC and HTTP+JSON/REST bindings at once. Add the `grpc` feature to serve the same agent state over gRPC too.
- **`rusty_a2a::signing`** (feature `signing`) — sign and verify an `AgentCard` with a JWS (RFC 7515) over its JSON Canonicalization Scheme representation (RFC 8785), per spec Section 8.4.

## Scope

The A2A spec defines three interoperable protocol bindings: **JSON-RPC 2.0**, **gRPC**, and **HTTP+JSON/REST** — this crate implements all three, client and server alike. The server (feature `server`) serves JSON-RPC and HTTP+JSON/REST from the same `axum::Router`/port; add the `grpc` feature and call `AgentServer::build()` to get an `AgentServices` handle that also serves gRPC (via `tonic`), sharing the same task store and executor across all bindings. The client (feature `client`) provides `A2aClient` (JSON-RPC), `client::RestClient` (HTTP+JSON/REST), and — with the `grpc` feature also enabled — `client::GrpcClient`, each a standalone type with the same method names and signatures; pick the one matching the interface your target agent declares, or use `*::discover`/`*::from_agent_card` to pick automatically. Per spec Section 5.1, an agent only needs to support the protocols it chooses to support; declaring a single `JSONRPC` interface in your Agent Card is fully compliant.

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
- Multi-tenant isolation: `TaskStore` scopes every task and push notification config by `tenant` (spec Section 4.2) — a task or config created under one tenant (or no tenant at all) is invisible to, unlistable by, and unmutable through a request naming a different tenant. All three bindings pass the caller's real `tenant` through to the store; REST reads it from a `/{tenant}` URL path prefix (spec's `additional_bindings`) if present, else the JSON body where an operation has one, else a `?tenant=` query parameter (`GET`s and the body-less `:cancel`/`:subscribe`/`DELETE` actions) — the path segment wins when more than one is given.
- Client-side REST and gRPC transports (`client::RestClient`, `client::GrpcClient`), matching `A2aClient`'s JSON-RPC API one-for-one — including SSE/gRPC streaming and reconstruction of the specific `A2aError` from each binding's wire error shape: exact for all three bindings when the error is one of the nine A2A-specific ones (JSON-RPC's numeric codes, and REST/gRPC's shared `google.rpc.ErrorInfo` `reason` string); a best-effort guess from the bare HTTP status/gRPC code otherwise, for the remaining generic categories (auth, validation, internal) that have no A2A-specific reason to carry.
- REST's `SubscribeToTask` is routed on its spec-literal `GET /tasks/{id}:subscribe` binding; `POST /tasks/{id}:subscribe` (this crate's original, non-spec-literal wiring) still works too, for backward compatibility. `client::RestClient` sends the `GET`, since another SDK's server has no reason to accept the `POST`.
- `mtls` security schemes can be satisfied via `AgentServer::with_mtls_header`: this crate's own servers never terminate TLS themselves, so there's no client certificate to inspect directly, but a deployment behind a TLS-terminating reverse proxy that already verified the client certificate and recorded the result in a header (nginx's `ssl-client-verify`, Envoy's `x-forwarded-client-cert`, an AWS ALB's `x-amzn-mtls-clientcert`, ...) can point the scheme at that header/gRPC-metadata entry instead, and your `AuthVerifier` decides what its value has to say to accept the request.
- `SendMessageConfiguration.taskPushNotificationConfig` registers push-notification delivery in the same request that creates a task, rather than requiring a separate `CreateTaskPushNotificationConfig` call afterward (which the client can't even make yet, since it doesn't know the server-assigned task id) - honored only on the turn that actually creates the task, so a continuation turn echoing the same inline config doesn't register a duplicate.
- Continuing a task across turns (e.g. answering `InputRequired` by sending a new message with `taskId` set) now records that turn's message into `task.history` too - previously only the very first turn's message was ever recorded, so `GetTask`/`ListTasks` under-reported multi-turn conversations after turn 1.
- `ListTaskPushNotificationConfigs.pageSize`/`pageToken` are honored (paginated in memory over the task's full config list, since a single task's configs are expected to be few) - previously always returned every config on one page with an empty `nextPageToken`, matching `ListTasks`' pagination now.
- `ContentTypeNotSupportedError`: a `Part.mediaType` this agent never declared via `AgentCard.defaultInputModes` is rejected before the executor runs. Checked at the agent level only - the protocol has no way to know which `AgentSkill` (whose `inputModes` can override the agent-wide defaults) a message is meant to invoke, so a skill-level override can't be applied. An agent that leaves `defaultInputModes` empty accepts anything, and a `Part` with no `mediaType` set makes no claim to check either way.
- Client-side Agent Card signature verification: `A2aClient::discover_and_verify`/`RestClient::discover_and_verify`/`GrpcClient::discover_and_verify` (feature `signing`) fetch the card and verify it against caller-supplied trusted `signing::VerifyingKey`(s) before returning it, failing closed with `ClientError::AgentCardSignatureInvalid` if the card is unsigned or none of its `signatures` verify against any trusted key. There's no default trust root - you must supply the key(s) you trust, e.g. pinned out-of-band. Plain `discover` (no verification) is unaffected and still available.
- A task-shaped `SendStreamingMessage`/`SubscribeToTask` stream now begins with the `Task` object itself (spec Sections 3.1.2/3.1.6), before any `TaskStatusUpdateEvent`/`TaskArtifactUpdateEvent`: for `SendStreamingMessage`, the just-created (or, for a continuation turn, current) task snapshot; for `SubscribeToTask`, the task's state at the moment of subscription. A `SubscribeToTask` reconnect via `Last-Event-ID` (this crate's own extension beyond the spec) skips this lead-in, since re-announcing state the caller already has would defeat the point of replaying only what was missed - gRPC has no such reconnect concept, so every gRPC subscribe (including a resubscribe) leads with a `Task`.
- `SendMessage`/`SendStreamingMessage` now reject a message continuing a task that's already in a terminal state (`Completed`/`Failed`/`Canceled`/`Rejected`) with `UnsupportedOperationError`, matching the same guard `CancelTask`/`SubscribeToTask` already had.
- Push notification webhook deliveries now POST a `StreamResponse` envelope (`{"task": {...}}`, spec Section 4.3.3), not a bare `Task` body.
- `ListTasks` sorts by status timestamp descending (most recently updated first, spec Section 3.1.4) - previously sorted by task id, which bears no relation to recency.
- `A2A-Version` is now validated identically on all three bindings (spec Sections 3.2.6/3.6.2/5.1) - previously only JSON-RPC checked it; REST and gRPC accepted any (or no) version.
- gRPC errors now carry a `google.rpc.ErrorInfo` detail with the same `reason` string JSON-RPC/REST already send (spec Section 10.6), letting `client::GrpcClient` reconstruct the exact `A2aError` instead of guessing from the bare gRPC code alone (several distinct A2aErrors share a code, e.g. `FailedPrecondition`).
- `SendMessage` now rejects a message with no `Part`s (spec Section 5.7/3.3.2) instead of proceeding to the executor with an empty history entry.
- Out-of-band `AUTH_REQUIRED` continuation (spec Section 7.6.1): a live `SendStreamingMessage`/`SubscribeToTask` stream now stays open across `TASK_STATE_AUTH_REQUIRED` instead of closing, so if an `AgentExecutor` keeps running after receiving a credential out-of-band (no new client message required), its later events still reach whoever's listening. `INPUT_REQUIRED`'s stream-closing behavior, and blocking `SendMessage` returning immediately at either interrupted state, are both unchanged.
- SSRF protection on push notification webhook URLs (spec Section 13.2, SHOULD): `AgentServer::with_webhook_ssrf_protection` rejects a webhook `url` that's a literal private/loopback/link-local IP address, or a hostname that resolves to one, both when a config naming it is registered and again right before each delivery (catching DNS rebinding too). Off by default - a local dev/test setup delivering to its own loopback webhook receiver is indistinguishable, from the wire alone, from an attacker probing this agent's own internal network, so only the deployer knows which situation applies.
- `Cache-Control`/`ETag` on the Agent Card discovery endpoint (spec Section 8.6.1, SHOULD): the `ETag` is derived from `AgentCard.version` and computed once at server construction; a request carrying a matching `If-None-Match` gets a bare `304 Not Modified`.
- The push notification HTTP client now has a request timeout (spec Section 13.2, SHOULD: 10-30s; this crate uses 20s), so a slow or unresponsive webhook can no longer hold a delivery attempt open indefinitely.
- Push notification delivery now retries with exponential backoff on failure (spec Section 13.2, SHOULD): up to 4 attempts total (1 initial + 3 retries, 200ms/400ms/800ms apart) for a network error or a 5xx/429 response - a receiver's own way of asking for a retry - but not for other 4xx responses, since resending an identical request can't change that outcome.
- Signed Agent Cards' JWS protected header now always includes `typ: "JOSE"` and a `kid` (spec Section 8.4.2, MUST) - when the caller doesn't supply a `kid`, one is deterministically derived from the signing key's own public material, so it's still always present and stable across every signature the same key makes.
- `GetExtendedAgentCard` now fails closed (spec Section 13.3, MUST require authentication unconditionally) when no `AuthVerifier` is configured, or the agent declares no `securitySchemes` to authenticate against - previously it silently served the card unauthenticated in either case. Its REST response also now carries `Cache-Control`/`ETag`, matching the base Agent Card.
- Push notification webhook URLs must now be `https` when `AgentServer::with_webhook_ssrf_protection` is enabled (spec Section 13.2, SHOULD) - bundled under the same opt-in flag as the existing SSRF IP-range check, since both guard the same delivery path.
- Server-authored `Message`s (a bare `SendMessage` reply, or a `TaskStatus.message`) now always carry `contextId` (and `taskId`, when a task was created) if the `AgentExecutor` didn't already set them (spec Section 4.1.4, MUST).
- `Part`, `SecurityScheme`, and `OAuthFlows` - all spec `oneof`s - now reject a JSON object with zero or more than one of their content keys on deserialize, instead of silently resolving to whichever variant is tried first (spec Sections 4.1.6/4.5.1/4.5.7, MUST "exactly one of").
- An `Artifact` update with no `parts` is now dropped (logged, not delivered) rather than reaching the wire (spec Section 4.1.7, MUST "at least one part").
- `pageSize`/`historyLength` outside their valid range (`pageSize`: 1-100; `historyLength`: non-negative) are now a validation error (`InvalidParams`) instead of being silently clamped/coerced into range (spec Sections 3.1.4/3.3.2).
- A `401 Unauthenticated` error now names the security scheme(s) that would have satisfied the request (spec Section 3.3.2, SHOULD "include authentication challenge information").
- The JSON-RPC binding now distinguishes `-32700` (invalid JSON) from `-32600` (syntactically valid JSON that isn't a valid Request object) instead of collapsing both into `-32700` (spec Section 9.5).
- The REST binding's own extractor rejections (malformed JSON body, an unparsable query parameter like `pageSize=notanumber`) now use the `google.rpc.Status` JSON envelope (spec Section 11.6) instead of `axum`'s default plain-text rejection response.
- Per-principal task authorization scoping (spec Section 13.1, MUST): `AuthVerifier::authorize_task` - a new method with a no-op default, so existing verifiers are unaffected - is called for every operation that touches a specific task (`GetTask`, `CancelTask`, `SubscribeToTask`, the push-notification-config CRUD family, and a `SendMessage`/`SendStreamingMessage` continuing an existing task) once a request has authenticated, letting your `AuthVerifier` decide whether the authenticated caller may access that particular task - this crate has no opinion on what "authorized" means for your deployment (exact ownership? role-based? team-shared?), the same reason it doesn't decide what a valid credential is either. `ListTasks` calls it per candidate task and silently omits whatever it rejects, rather than failing the whole call, per the spec's "MUST only return tasks visible to the authenticated client" - internally fetching more than one store page if needed to still fill a full requested page of authorized results.
- `Engine::new` now logs a `tracing::warn!` if the `AgentCard` declares an `apiKey` security scheme with `location: "query"` or `"cookie"` alongside a non-REST interface (JSON-RPC or gRPC). The spec's "Equivalent Authentication" requirement (Section 5.1) has no defined query/cookie credential transport for those bindings - Sections 7.3/9.2/10.2 only ever describe headers/metadata - so such a scheme is realistically unsatisfiable there; this is a loud diagnostic for a likely-unintended `AgentCard`, not a rejection, since the spec doesn't actually forbid the combination.

Not implemented (contributions welcome):

- Native TLS termination: this crate's own servers (`AgentServer::serve`, `AgentServices::serve_grpc`) always bind plain TCP, so `mtls` (above) only works fronted by an external TLS-terminating proxy - there's no `serve_https`/built-in client-certificate verification for verifying one directly inside this crate.

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

`client::RestClient::discover`/`client::GrpcClient::discover` (the latter needs the `grpc` feature too) work exactly the same way, for an agent whose Agent Card prefers the `HTTP+JSON` or `GRPC` binding instead.

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

`tests/integration.rs` spins up a real `AgentServer` on a local port and drives it with a real `A2aClient` and a bare `reqwest::Client`, covering the full task lifecycle, streaming, non-blocking sends, cancellation, push notification config CRUD, and the REST binding's routing/error shape, over both bindings sharing one task store. `tests/grpc_integration.rs` does the same against `AgentServices::serve_grpc` with a real generated `tonic` client. `tests/rest_client.rs` and `tests/grpc_client.rs` cover the same lifecycle again, this time through `client::RestClient`/`client::GrpcClient` themselves rather than a bare `reqwest`/`tonic` client, including each one's error-shape reconstruction back into the specific `A2aError` and cross-checking against `A2aClient` that all bindings share one task store. `tests/security_and_push_notifications.rs` covers `AuthVerifier` enforcement (accepted/rejected/misconfigured-fail-closed, across JSON-RPC and REST, plus the `GetExtendedAgentCard` auth gate) and push notification delivery to a real local webhook receiver. `tests/history_length_and_extensions.rs` covers `historyLength` truncation on `SendMessage` and required-extension enforcement across JSON-RPC and REST. `tests/subscribe_replay.rs` and `tests/subscribe_replay_grpc.rs` drive a `Notify`-gated agent to deterministically disconnect and reconnect mid-stream, covering `Last-Event-ID` replay on JSON-RPC/REST, the idle-interrupted-task replay-then-snapshot path, gRPC's coarser whole-buffer replay, and that a fresh (non-reconnect) subscribe always leads with a `Task` snapshot first. `tests/tenant_isolation.rs` and `tests/tenant_isolation_grpc.rs` drive two differently-tenanted clients against the same server, covering cross-tenant `GetTask`/`ListTasks`/`CancelTask`/push-config isolation, REST's `?tenant=` query parameter, and that omitting `tenant` is a single consistent shared namespace (so a single-tenant deployment's behavior is unchanged). `tests/rest_tenant_path_bindings.rs` covers the REST binding's other way to send `tenant` - the `/{tenant}` URL path prefix (`additional_bindings`) - across every route, including that the path segment wins when a request also sends a conflicting `tenant` via the body or `?tenant=`, and that a tenant literally named like a literal route segment (e.g. `"tasks"`) doesn't break routing. `tests/mtls_proxy_header.rs` and `tests/mtls_proxy_header_grpc.rs` cover `AgentServer::with_mtls_header` across all three bindings: a request carrying the configured header's expected value is accepted, one carrying a different value or no header at all is rejected, and - critically - without `with_mtls_header` configured at all, even a request that happens to carry the exact header a proxy would set is still rejected, confirming the scheme stays inert by default. `tests/multi_turn_history.rs` covers a continued task's history retaining both turns' messages, not just the first turn's. `tests/security_and_push_notifications.rs` additionally covers `SendMessageConfiguration.taskPushNotificationConfig` registering push delivery inline at task creation, that a continuation turn echoing the same inline config doesn't register a duplicate, and `ListTaskPushNotificationConfigs.pageSize`/`pageToken`. `tests/content_type_validation.rs` covers `ContentTypeNotSupportedError`: a `Part.mediaType` outside `AgentCard.defaultInputModes` is rejected, a declared one or a part with no `mediaType` at all is accepted, and an agent that leaves `defaultInputModes` empty accepts anything. `tests/client_jws_verification.rs` covers `discover_and_verify` across all three clients: a correctly-signed card verifies and discovery succeeds, an unsigned card is rejected (while plain `discover` against the same card still succeeds), a card signed only by an untrusted key is rejected, and a card carrying multiple signatures verifies as long as any one of them matches a trusted key. `tests/terminal_task_rejection.rs` covers `SendMessage`/`SendStreamingMessage` rejecting a continuation to an already-`Completed` task with `UnsupportedOperationError`, across JSON-RPC and REST. `tests/list_tasks_ordering.rs` covers `ListTasks` returning tasks most-recently-updated first. `tests/version_negotiation.rs` covers `A2A-Version` enforcement on REST (including `GetExtendedAgentCard`) and gRPC, mirroring `tests/integration.rs`'s existing JSON-RPC coverage. `tests/grpc_error_details.rs` covers the gRPC binding's `google.rpc.ErrorInfo` detail letting `client::GrpcClient` reconstruct the exact `A2aError` for two errors that share the same gRPC code (`TaskNotCancelable` and `UnsupportedOperation`, both `FailedPrecondition`) rather than only one of them being guessable. `tests/auth_required_continuation.rs` covers out-of-band `AUTH_REQUIRED` continuation: a `Notify`-gated agent that keeps running after `AUTH_REQUIRED` (simulating a credential arriving out-of-band) delivers its eventual completion on the same still-open `SendStreamingMessage`/`SubscribeToTask` stream, with no reconnect or follow-up message; a separate test confirms blocking `SendMessage` is unaffected and still returns immediately at `AUTH_REQUIRED`. `tests/webhook_ssrf_protection.rs` covers `AgentServer::with_webhook_ssrf_protection`: a literal private/loopback/link-local webhook URL is rejected at registration time (both via `CreateTaskPushNotificationConfig` and the inline `taskPushNotificationConfig` on `SendMessage`) when enabled, a public-looking address is still accepted, and - critically - a loopback URL is accepted by default when the feature isn't opted into. `tests/agent_card_caching.rs` covers the Agent Card endpoint's `Cache-Control`/`ETag` headers, a matching `If-None-Match` getting a bodyless `304`, and a stale `If-None-Match` still getting the full card. `tests/webhook_retry.rs` covers push notification delivery retrying with exponential backoff and succeeding once the webhook recovers, giving up after exactly 4 attempts against a persistently-failing endpoint, and not retrying at all against a 4xx response. `tests/extended_agent_card_auth.rs` covers `GetExtendedAgentCard` failing closed (spec Section 13.3's unconditional auth MUST) both with no `AuthVerifier` configured and with a verifier but no `securitySchemes` declared, succeeding once both are in place, and its REST response carrying `Cache-Control`/`ETag`. `tests/input_validation.rs` covers `pageSize`/`historyLength` out-of-range rejection, an empty-`parts` `Artifact` never reaching the wire, the JSON-RPC binding's `-32700`/`-32600` distinction, and the REST binding's extractor rejections (malformed JSON body, unparsable query parameter) still using the `google.rpc.Status` envelope. `tests/server_message_ids.rs` covers server-authored `Message`s (a bare reply, and a task status message) carrying `contextId`/`taskId` per spec Section 4.1.4. `tests/task_authorization_scoping.rs` covers `AuthVerifier::authorize_task`: two callers authenticated against the same verifier can't read, cancel, continue, or manage push-notification configs for each other's tasks, `ListTasks` silently omits (rather than fails on) tasks the caller isn't authorized to see even when they're interleaved with authorized ones across multiple internal store pages, and a public agent (or a verifier that doesn't implement the new hook) is unaffected.

Only the gRPC paths need a `protoc` binary on `PATH`: each suite's `required-features` names the minimum it actually uses, so the JSON-RPC and REST suites run under `--features client,server` alone. A gate that asks for more than its target needs is invisible — cargo drops the target and the suite "passes" by not existing — so CI asserts each suite resolves under its own feature set.

## License

Apache-2.0.
