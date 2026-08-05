# rusty-acp

[![CI](https://github.com/baileyrd/rusty_acp/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_acp/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rustc-1.86%2B-orange.svg)](#install)

A Rust implementation of the [Agent Communication Protocol][acp] (ACP) **v0.2.0** — the open,
REST-based standard for making AI agents interoperable across frameworks, languages and
organisations.

The crate gives you three layers, each usable on its own:

| Layer | Feature | What it does |
| --- | --- | --- |
| `rusty_acp::types` | always on | The complete wire format — manifests, messages, runs, events, sessions, errors — round-tripping through the protocol's exact JSON. |
| `rusty_acp::client` | `client` | An HTTP client for calling **any** ACP server, in any language, including SSE streaming. |
| `rusty_acp::server` | `server` | An [`axum`] router that hosts **your** agents behind the standard endpoints. |
| `rusty_acp::server::store` | `redis-store` | A Redis-backed store, for several replicas behind a load balancer. |
| open discovery | `well-known` | Serves agent metadata as YAML at `/.well-known/agent.yml`. |

Both directions speak the same protocol, so a Rust agent is a drop-in peer for a Python (BeeAI),
TypeScript, LangChain or CrewAI one.

## Install

Not published to crates.io — depend on it from git:

```toml
[dependencies]
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp" }
```

Add `rev = "<commit>"` to pin a reproducible build. Without one, Cargo tracks the default
branch and picks up whatever has landed there on the next `cargo update`.

Default features are `client` + `server`. Take just what you need — the feature names are
the ones in the table above:

```toml
# types only
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", default-features = false }

# one layer, without the other
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", default-features = false, features = ["client"] }

# + Redis-backed HA, or open discovery
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", features = ["redis-store"] }
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", features = ["well-known"] }
```

Minimum supported Rust version is **1.86**, verified in CI on every change. The optional
`redis-store` feature requires **1.88**, since the `redis` crate's own floor is higher; an
optional dependency does not raise the MSRV for everyone else.

## Serve an agent

```rust
use rusty_acp::server::{AcpServer, Agent, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Error};

struct Echo;

#[async_trait::async_trait]
impl Agent for Echo {
    fn manifest(&self) -> AgentManifest {
        AgentManifest::new(AgentName::new("echo")?, "Echoes the input back")
    }

    async fn run(&self, ctx: RunContext) -> Result<(), Error> {
        ctx.reply_text(ctx.input_text()).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let router = AcpServer::builder().agent(Echo).build()?.into_router();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    axum::serve(listener, router).await?;
    Ok(())
}
```

That single registration serves every endpoint in the specification. For one-off agents,
`agent_fn` takes a closure instead of a trait impl:

```rust
use rusty_acp::server::agent_fn;
use rusty_acp::types::{AgentManifest, AgentName};

let echo = agent_fn(
    AgentManifest::new(AgentName::new("echo")?, "Echoes the input back"),
    |ctx| async move {
        ctx.reply_text(ctx.input_text()).await?;
        Ok(())
    },
);
```

Because the result is a plain `axum::Router`, ordinary tower middleware — auth, CORS, tracing,
rate limiting — layers on top as usual.

## Call an agent

```rust
use rusty_acp::client::AcpClient;
use rusty_acp::types::Message;

let client = AcpClient::new("http://localhost:8000")?;

// Discovery
for manifest in client.list_all_agents().await? {
    println!("{}: {}", manifest.name, manifest.description);
}

// Synchronous: block until the run settles
let run = client.run_sync("echo", [Message::user("hello")]).await?;
println!("{}", run.output_text());

// Asynchronous: start now, poll later
let started = client.run_async("echo", [Message::user("hello")]).await?;
let finished = client.wait_for_run(started.run_id, WaitOptions::default()).await?;
```

### Streaming

```rust
use futures_util::StreamExt;
use rusty_acp::types::Event;

let mut stream = client.stream("writer", [Message::user("hello")]).await?;
while let Some(event) = stream.next().await {
    match event? {
        Event::MessagePart { part } => print!("{}", part.as_text().unwrap_or_default()),
        Event::RunCompleted { run } => println!("\ndone in {:?}", run.finished_at),
        _ => {}
    }
}
```

The stream ends **after** the terminal event, so the final `run.*` snapshot is never lost to the
cut-off. `client::collect_run` drains a stream straight into that final `Run`.

On the server side, stream output part by part:

```rust
let mut writer = ctx.begin_message().await?;
for token in tokens {
    writer.push_text(token).await?;
}
writer.finish().await?;
```

### Pausing for client input (await / resume)

An agent can stop mid-run to ask a question. The run moves to `awaiting` with an
`await_request`; the client answers with `POST /runs/{run_id}`.

```rust
// Agent side
let resume = ctx.await_json(serde_json::json!({ "question": "What is your name?" })).await?;
let name = resume.as_value()["answer"].as_str().unwrap_or("stranger");
ctx.reply_text(format!("Hello, {name}!")).await?;
```

```rust
// Client side
let paused = client.run_sync("greeter", [Message::user("hi")]).await?;
assert_eq!(paused.status, RunStatus::Awaiting);

let done = client
    .resume_run(RunResumeRequest::new(
        paused.run_id,
        serde_json::json!({ "answer": "Ada" }).into(),
        RunMode::Sync,
    ))
    .await?;
```

`AwaitRequest`/`AwaitResume` also carry typed payloads via `from_value` and `deserialize`.

### Cancellation

`POST /runs/{run_id}/cancel` returns `202` and the run moves to `cancelling`. The agent's future
is dropped, and long-running agents can react promptly by selecting on `ctx.cancelled()`.

Cancellation is *accepted* before it is *applied* — and with several replicas it may be accepted
by one replica and applied by another — so the snapshot in the `202` can still read `in-progress`:

```rust
// Polls until the run is terminal, giving up after WaitOptions::timeout.
let cancelled = client.cancel_and_wait(run_id, WaitOptions::default()).await?;
```

### Sessions

Pass a `session_id` to chain runs into a conversation. The server records every input and output
message and exposes them as dereferenceable URLs, matching ACP's [distributed sessions][sessions]
design — history is a list of *links*, so a session can span several servers.

```rust
let session_id = SessionId::new();
client.create_run(RunCreateRequest::new(name, [Message::user("first")]).with_session_id(session_id)).await?;
client.create_run(RunCreateRequest::new(name, [Message::user("second")]).with_session_id(session_id)).await?;

let session = client.get_session(session_id).await?;
let messages = client.fetch_session_history(&session).await?;   // follows every URL, local or remote
```

Inside an agent, `ctx.history()` gives the messages this server already holds, and
`ctx.session()` gives the full link list for anything hosted elsewhere.

### Stateful agents

Replaying the whole history every turn gets expensive. An agent can instead persist its own
state for the session — a summary, accumulated preferences, a working set:

```rust
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Memory { summary: String, turns: u32 }

let mut memory: Memory = ctx.load_state().await?.unwrap_or_default();
memory.turns += 1;
memory.summary = summarize(&memory.summary, ctx.input_text());
ctx.store_state(&memory).await?;
```

State is scoped to the session, shared by every run in it, and survives across replicas.
Following ACP's model, `Session.state` holds a *link* to the document rather than the document
itself, so `GET /session/{id}` stays small however large the state grows.

### Artifacts

An artifact is a named output — a file, image, or structured result — that a client can offer
for download or render richly. ACP defines no separate type: an artifact is simply a message
part with a `name`.

```rust
ctx.reply_artifact("result.json", "application/json", r#"{"ok": true}"#).await?;

// Binary content is encoded and declared together, so the two can't drift apart:
ctx.reply_part(MessagePart::binary_artifact("chart.png", "image/png", png_bytes)).await?;
```

`MessagePart::decoded_content()` reverses it on the receiving side, undoing base64 when that is
the declared encoding.

### Open discovery

With the `well-known` feature, the server also publishes its manifests as YAML at
`/.well-known/agent.yml`, so a crawler or another agent can find what a domain hosts without
knowing the ACP endpoints. The content is built from the same manifests `GET /agents` serves,
so the two cannot drift.

## Running several replicas

Runs live in process memory by default, which is right for a single agent host.
For the multi-replica setup ACP's [high-availability guide][ha] describes — identical
replicas behind a load balancer, no session affinity — give every replica the same
shared store:

```rust
use rusty_acp::server::{store::RedisStore, AcpServer};

let store = RedisStore::connect("redis://127.0.0.1/").await?;

let router = AcpServer::builder()
    .agent(my_agent)
    .store(std::sync::Arc::new(store))
    // Every replica advertises the same public address, so session history
    // links stay valid whichever replica wrote them.
    .base_url("https://acp.example")
    .build()?
    .into_router();
```

That is the whole change. Every endpoint reads and writes through the store, so once
it is shared:

- `GET /runs/{id}`, `GET /runs/{id}/events` and `GET /session/{id}` are served by **any** replica.
- `POST /runs/{id}` (resume) and `POST /runs/{id}/cancel` accepted by any replica are **routed to
  whichever replica is executing the agent**.
- Resuming with `mode: stream` against one replica streams events emitted by the agent
  running inside another.

### How it works

Two ideas carry the design:

**The replica executing a run is its sole writer.** Everyone else reads snapshots and
sends control signals. That is what lets `put_run` be a plain overwrite — there is never
a second writer to race with, so no distributed locking is needed.

**One channel does both jobs.** A `Notification` is either an `Event` fanning *out* to
streaming clients on any replica, or a `Resume`/`Cancel` routing *in* to the executing
replica. A backend therefore needs exactly one pub/sub primitive: a Redis channel, a
Postgres `LISTEN`/`NOTIFY` channel, or an in-process broadcast.

### When a replica dies

The sole-writer invariant has a weak point: a writer can die. Without something
watching, the run it was executing would stay non-terminal forever, with nothing left
to consume a resume, apply a cancel, or write a terminal state.

Each executing replica therefore holds a short **lease** on its runs and keeps renewing
it. A non-terminal run whose lease has lapsed has demonstrably lost its writer, so the
next replica asked about it marks it `failed` and publishes `run.failed` — which
unblocks streaming and `sync` callers on every replica.

```rust
AcpServer::builder()
    .replica_id("agent-host-7")           // shows up as the lease owner in logs
    .lease_ttl(Duration::from_secs(30))   // window between death and reaping
    .sync_timeout(Duration::from_secs(300))
```

Two deliberate choices:

- **Failed, not retried.** Re-running elsewhere would repeat whatever output and side
  effects the run already produced, and ACP promises no idempotency. Taking over a run
  is only safe for agents that opt in, which is not something the protocol lets us know.
- **Reaped lazily, on read.** No background sweeper: the check costs one lease lookup on
  reads that were already hitting the store, and `sync` waiters re-check every lease
  interval so they self-heal rather than waiting out their timeout.

Waits are bounded on both sides. `sync` returns the run as it stands after
`sync_timeout` rather than hanging — so check `status` rather than assuming terminal —
and the client's `wait_for_run` / `cancel_and_wait` take `WaitOptions` with a deadline:

```rust
let run = client.wait_for_run(run_id, WaitOptions::default()).await?;
let run = client.wait_for_run(run_id, WaitOptions::default().without_timeout()).await?;
```

### Recovering a lost run, instead of just failing it

Failing an abandoned run is always correct but unambitious — the work is lost and the
client resubmits. An agent that declares itself replayable gets more: when its replica
dies, the server starts a **replacement run** and links the two.

```rust
let summarize = agent_fn(manifest, |ctx| async move { /* ... */ })
    .with_recovery();            // or `fn recoverable(&self) -> bool { true }`
```

```text
run A: failed     error.data.replaced_by = <run B>
   └── run B: running    generic event { replaces: <run A>, attempt: 2 }
```

The abandoned run keeps its own history and stays failed. Nothing already streamed to a
client is retracted, and no run ends up with two sets of output — which is exactly why
this is a *new* run rather than a re-execution in place. Both links use the
specification's own extension points: `Error.data` on the failed run, a `generic` event
on the replacement.

Three caveats:

- **The default is off, deliberately.** Replaying an agent that takes a payment or sends
  a message repeats it. ACP carries no idempotency contract, so the server cannot work
  out which agents are safe — it has to be told.
- **Every replica must host the same agents**, and share `max_recovery_attempts`. The
  replica that notices an abandoned run is the one that re-runs it; if it does not have
  that agent registered, the run is failed as usual.
- **There is an attempt ceiling** (default 3), so a run that kills whatever executes it
  cannot migrate around the fleet forever.

### Writing your own backend

Implement [`Store`](https://github.com/baileyrd/rusty_acp/blob/main/src/server/store/mod.rs) —
run snapshots, the event log, sessions, ownership leases and per-run pub/sub. The
trait documents the invariants a backend may rely on and the two it must provide
(subscription liveness on return, and atomic session appends). `InMemoryStore` and
`RedisStore` are both implemented against exactly that contract, and the multi-replica
test suite runs unchanged against either.

[ha]: https://agentcommunicationprotocol.dev/how-to/high-availability

## Protocol coverage

Every endpoint and schema in the ACP v0.2.0 [OpenAPI document][openapi] is implemented.

| Endpoint | Server | Client |
| --- | --- | --- |
| `GET /ping` | ✅ | `ping` |
| `GET /agents` (`limit`, `offset`) | ✅ | `list_agents`, `list_all_agents` |
| `GET /agents/{name}` | ✅ | `get_agent` |
| `POST /runs` — `sync` / `async` / `stream` | ✅ | `create_run`, `run_sync`, `run_async`, `stream_run` |
| `GET /runs/{run_id}` | ✅ | `get_run`, `wait_for_run` |
| `POST /runs/{run_id}` (resume) | ✅ | `resume_run`, `stream_resume` |
| `POST /runs/{run_id}/cancel` | ✅ | `cancel_run`, `cancel_and_wait` |
| `GET /runs/{run_id}/events` | ✅ | `list_run_events` |
| `GET /session/{session_id}` | ✅ | `get_session`, `fetch_session_history` |

Plus resource endpoints backing the links ACP puts in a session — `GET /session/{id}/messages/{i}`
for history and `GET /session/{id}/state` for state — and, with the `well-known` feature,
`GET /.well-known/agent.yml` for open discovery. None of those are in the OpenAPI document: the
spec says history and state are URLs on resource servers, and leaves where they point to the
implementation.

Also covered:

- **All seven run states** — `created`, `in-progress`, `awaiting`, `cancelling`, `cancelled`,
  `completed`, `failed` — with terminal transitions applied exactly once, so a cancellation
  racing a completion cannot rewrite the outcome.
- **All eleven event types**, tagged on the wire exactly as the spec names them
  (`message.created`, `message.part`, `message.completed`, `generic`, `run.created`,
  `run.in-progress`, `run.awaiting`, `run.completed`, `run.failed`, `run.cancelled`, `error`).
- **Multimodal messages** — any MIME type, `plain` or `base64` encoding, inline `content` or a
  `content_url`, with the `content`/`content_url` exclusivity rule enforced.
- **Part metadata** — `CitationMetadata` and `TrajectoryMetadata`, discriminated by `kind`.
- **The full agent manifest** — capabilities, dependencies, links, authors, tags, licensing and
  runtime status metrics.
- **Validated identifiers** — `AgentName` enforces the RFC 1123 DNS label rules; `Role` enforces
  the `user` / `agent` / `agent/{name}` pattern, both at parse time.
- **The error model** — `server_error`, `invalid_input` and `not_found` mapped to HTTP 500, 422
  and 404 in both directions.
- **Stateful agents** — `load_state`/`store_state` scoped to the session, exposed as a link.
- **Artifacts** — named outputs with base64 handled for binary content.

Two things the spec mentions are deliberately not implemented, because there is nothing stable to
implement against: **embedded/offline discovery** via container labels (the spec states the label
format is not standardized) and **registry-based discovery** (stated as "not yet part of official
ACP spec").

## Examples

```sh
cargo run --example echo_server      # two agents: one plain, one streaming
cargo run --example awaiting_agent   # pauses mid-run to ask a question
cargo run --example client_demo      # drives every client operation against a running server
cargo run --example ha_server        # two replicas sharing one store
```

Each example's header comment carries the equivalent `curl` invocations.

## Tests

```sh
cargo test --all-features
```

101 tests: wire-format round-trips for every schema, end-to-end coverage of discovery, all three
run modes, streaming order and aggregation, await/resume, cancellation of both running and
awaiting runs, session continuity and the error paths — plus a multi-replica suite that starts
two servers sharing one store and drives a run through one while observing, resuming and
cancelling it through the other — including killing a replica's whole runtime mid-run and
asserting the run gets reaped rather than hanging, and that a replayable one is replaced
by a fresh linked run.

The multi-replica suite runs against **both** backends. The Redis half is skipped unless
`ACP_TEST_REDIS_URL` is set; when it *is* set, an unreachable Redis fails the run rather than
quietly skipping:

```sh
ACP_TEST_REDIS_URL=redis://127.0.0.1:6379 cargo test --all-features
```

CI runs the suite on stable, beta and the 1.86 MSRV against a real Redis service, plus
`rustfmt`, `clippy -D warnings`, each feature combination built alone, a nightly `cargo doc`
with `-D warnings`, and `cargo package`.

## Notes on the server

- The default store holds runs in memory, capped by `AcpServerBuilder::max_runs` (default 1024).
  Active runs are never evicted; the oldest terminal ones go first. `RedisStore` expires keys on a
  configurable TTL instead (default 24h), as the HA guide calls for.
- Emitting is `async`: every emit writes to the store and publishes to its subscribers. With the
  default store that is nearly free; with a shared backend it is a network write that can fail,
  and `?` is what turns a storage outage into a failed run rather than a silently truncated one.
- Session history URLs are built from `AcpServerBuilder::base_url` when set, otherwise from the
  request's `Host` header (honouring `X-Forwarded-Proto` / `X-Forwarded-Host`). Set `base_url`
  explicitly behind a proxy that rewrites paths.
- An input part whose `content_type` the agent's manifest does not accept is rejected with
  `invalid_input` before the agent runs.

## License

Apache-2.0. See [LICENSE](LICENSE).

[acp]: https://agentcommunicationprotocol.dev
[openapi]: https://github.com/i-am-bee/acp/blob/main/docs/spec/openapi.yaml
[sessions]: https://agentcommunicationprotocol.dev/core-concepts/distributed-sessions
[`axum`]: https://docs.rs/axum
