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
| `rusty_acp::server::store` | `postgres-store` | A Postgres-backed store: the same, with history that outlives a key expiry and is queryable. |
| open discovery | `well-known` | Serves agent metadata as YAML at `/.well-known/agent.yml`. |
| metrics | `metrics` | Records run, lease and store metrics through the [`metrics`] facade. |

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

# + a shared store for HA, or open discovery
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", features = ["redis-store"] }
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", features = ["postgres-store"] }
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", features = ["well-known"] }
rusty-acp = { git = "https://github.com/baileyrd/rusty_acp", features = ["metrics"] }
```

Minimum supported Rust version is **1.86**, verified in CI on every change. The optional
`redis-store` and `postgres-store` features require **1.88**, since `redis` and `sqlx` have
higher floors of their own; an optional dependency does not raise the MSRV for everyone else.

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
rate limiting — layers on top as usual. [Authentication](#authentication) has two ACP-specific
traps in it and gets its own section below.

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

#### Watching a run you did not start

`stream` and `stream_run` submit a run and stream it. To watch one that is already going —
started with `run_async`, or by somebody else entirely — attach to its log:

```rust
let started = client.run_async("writer", [Message::user("hello")]).await?;

let mut events = client.attach(started.run_id).await?;
while let Some(event) = events.next().await {
    println!("{:?}", event?);
}

// Or from after what you have already read:
let rest = client.attach_after(run_id, 41).await?;
```

Everything already in the log is replayed, then the stream continues live and ends after the
terminal event — the same sequence `stream_run` yields. Attaching to a run that has already
finished replays the whole log and closes, which is the useful answer rather than an error.

Attaching is in one respect *more* robust than `stream_run`: the run id is known before the first
byte arrives, where `stream_run` learns it from the first `run.*` event. Resumption needs that id,
so a connection dropping before any event arrives can be recovered here and cannot be there.

#### Surviving a dropped connection

A streaming run routinely outlives the connection carrying it — proxies time idle connections
out, load balancers recycle them, replicas die. Every streamed event is tagged with its index in
the run's durable log, so a stream that drops is picked up from the last event the client saw
rather than restarted or abandoned:

```rust
let client = AcpClient::builder("http://localhost:8000")
    .reconnect(ReconnectPolicy { max_attempts: 5, ..Default::default() })  // the default
    .build()?;

let client = AcpClient::builder("http://localhost:8000")
    .reconnect(ReconnectPolicy::disabled())   // a dropped stream simply ends
    .build()?;
```

Nothing changes at the call site: `stream`, `stream_run` and `stream_resume` yield the same
gapless sequence whether or not the connection survived it. The attempt ceiling counts
*consecutive* failures and is reset by any event that arrives, so a long run that drops
repeatedly while still making progress is not cut off.

Other clients can do the same thing directly, since it is ordinary SSE:

```sh
curl -N -H 'Accept: text/event-stream' -H 'Last-Event-ID: 41' \
  http://localhost:8000/runs/$RUN_ID/events
```

The server subscribes before it reads the log, which is what makes the splice gapless — anything
appended after the read arrives live, anything before it is in the read. That leaves an overlap
rather than a gap, and the index is what removes it exactly. Resuming a run that has already
finished replays the log and closes.

#### Riding out a transient failure

The same fleet that drops streams also drops ordinary requests: a balancer returns 502 mid-deploy,
a replica still starting returns 503, a connection is recycled between the request and the
response. The client retries these itself, with exponential backoff and jitter, honouring
`Retry-After` when the server sends one:

```rust
let client = AcpClient::builder("http://localhost:8000")
    .retry(RetryPolicy { max_retries: 3, ..Default::default() })   // the default
    .build()?;

let client = AcpClient::builder("http://localhost:8000")
    .retry(RetryPolicy::disabled())   // every failure reaches the caller
    .build()?;
```

Retried: connect errors, timeouts, 429, 502, 503 and 504. **Not 500** — that is what a server
returns when the *agent* failed, which a second attempt reproduces rather than resolves.

**Creating or resuming a run is not retried by default**, and reads and cancellations are. The
asymmetry is deliberate: a submission that timed out may well have been received and started, and
ACP has no idempotency key that would let a retry collapse into the first attempt — so retrying can
leave two runs behind, each with the side effects of one. Reading twice costs a round trip, and
cancelling a run that is already cancelling is a no-op. Callers whose agents are idempotent, or who
would rather have a duplicate run than a failed submission, can opt in:

```rust
.retry(RetryPolicy { retry_run_submission: true, ..Default::default() })
```

`wait_for_run` and `cancel_and_wait` treat a transient failure mid-wait as *not settled yet* rather
than as an answer, and keep polling until their deadline — a wait that exists to outlast a slow run
should not be ended by one bad poll. If the deadline arrives with the failure still happening, that
failure is what is reported, not a timeout that would hide it. Switching retrying off switches this
off too, so `RetryPolicy::disabled()` means one thing everywhere.

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

Being reachable without credentials is the point, so it needs exempting from anything you layer
in front of the server — see [Authentication](#authentication).

### Authentication

The crate takes no position on the scheme — building one in would mean picking one for
everybody — and the router is a plain `axum::Router`, so a tower layer is all it takes.
`cargo run --example authenticated_server` is a worked bearer-token setup, server and client.

What is not generic is **which endpoints must stay open**:

- **`/ping`** is the liveness check and **`/ready`** the readiness one, both probed by a load
  balancer that has no credentials. Behind a token, every replica reads as unhealthy — an outage,
  caused by an exemption list. `/ready` is the worse of the two to forget: a 401 there is
  indistinguishable from "do not send me traffic".
- **`/.well-known/agent.yml`** is *open discovery*. Being readable by an unauthenticated crawler
  is the whole purpose. A token in front of it does not secure it; it deletes it.

`GET /agents` is not on that list even though it serves the same manifests. The well-known
document is the public advertisement, `/agents` is the API — which is why ACP defines both.

The second trap is **session URLs**. A session's history is a list of dereferenceable URLs that
the client *follows*, one authenticated request per entry, and `fetch_session_history` will
follow them across servers. Put credentials on the `reqwest::Client` and they travel with
whatever it fetches:

```rust
use rusty_acp::reqwest;

let http = reqwest::Client::builder().default_headers(headers).build()?;
let client = AcpClient::with_http_client("http://localhost:8000", http)?;
```

`reqwest` is re-exported for this, so there is no second dependency to add and no version to match
by hand — get that wrong and the error is a mismatch between two types with the same name. It does
pin you to this crate's `reqwest` major version, which is the constraint that already existed,
now stated rather than discovered.

A scheme scoped to the *caller* rather than the *resource* — one-time nonces, per-replica
secrets, tokens audience-bound to a single host — breaks as soon as a session's URLs point at a
server the follower cannot authenticate to, which is the ordinary case once sessions are shared
across replicas. Whatever guards those URLs has to be satisfiable by whoever follows them.

One smaller detail: ACP defines three error codes — `server_error`, `invalid_input`,
`not_found` — and none of them means "unauthenticated". Return ordinary HTTP 401 rather than
dressing it up as an ACP error, and the client reports `AcpError::Http { status: 401 }` instead
of an `AcpError::Protocol` carrying a code that lies. It is also not retried, which is correct:
a 401 is a verdict, not a blip.

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

### Choosing a backend

Two shared stores ship with the crate. They implement the same trait and pass the same
multi-replica suite, so the choice is about what happens to a run *after* it finishes:

| | `RedisStore` | `PostgresStore` |
| --- | --- | --- |
| Expiry | A key TTL, 24h by default — the HA guide's model | None. `sweep()` deletes finished runs and idle sessions past a configured retention, and only when you call it |
| History | Gone when the TTL lapses | Kept until you decide otherwise |
| Queries | By run id only | Ordinary SQL: which runs failed today, which agent is busiest, what a session contained |
| Setup | None | Tables are created on connect |

The default `InMemoryStore` bounds both halves of what it holds: `max_runs` runs (evicting the
oldest **terminal** ones; active runs are never evicted) and `max_sessions` sessions (evicting the
least recently *used*, along with its state document). Both default to 1024.

```rust
AcpServer::builder().max_runs(4096).max_sessions(4096)
```

An evicted session is indistinguishable from one that never existed, so an agent's conversation
silently starts over — the same thing `RedisStore`'s TTL does, and logged at `warn` for exactly
that reason. A session in active use is by definition recently touched, but it is not *pinned*: a
long run with more than `max_sessions` fresh sessions started during it can still lose its
history. Raise the limit if sessions churn faster than runs complete.

```rust
use rusty_acp::server::store::PostgresStore;

let store = PostgresStore::connect("postgres://localhost/acp").await?;
```

Retention is **off by default**, since unbounded history is usually the reason to reach for
Postgres in the first place. Turn it on with `PostgresStoreConfig::retention` and call
`sweep()` from a job you control — nothing deletes anything on its own.

`sweep()` collects sessions on the same window, and returns both counts:

```rust
let swept = store.sweep().await?;
swept.runs       // finished runs, with their events, leases and recovery records
swept.sessions   // conversations, with their history and state documents
```

A session is stale when nothing has **written** to it since the cutoff — adopted it, appended to
it, or stored its state. Not when it was last *read*: turning a read into a write would put a row
lock in front of every run loading its own history, and a conversation being read but never added
to is one nobody is continuing. This differs from `InMemoryStore`, where a read does count as use,
because that store has to choose a victim among live sessions under a count bound, while this one
only has to answer whether a session is old.

A session with a run still in flight is never collected, however far past the window it sits, so a
sweep cannot take a conversation out from under the run about to append to it. Everything else that
goes leaves nothing behind — the same silent restart as a Redis TTL or an in-memory eviction, and
logged at `warn` for the same reason.

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

Losing the replica a caller happened to be talking to is the ordinary case here rather than an
outage, which is why the client [retries transient failures](#riding-out-a-transient-failure)
instead of surfacing them.

### Bounding how long a run waits for an answer

A run parked `awaiting` a client is cheap but not free: it holds a task, a run entry the default
store will never evict, and a lease its replica keeps renewing every few seconds. Nothing else
reclaims it — a non-terminal run with a live lease is indistinguishable from one that is working.

So conversations are bounded by default, at an hour:

```rust
AcpServer::builder()
    .await_timeout(Duration::from_secs(600))   // shorter
    .without_await_timeout()                   // or open-ended
```

Past the deadline the run is **failed**, with a message saying so rather than a bare
`server_error` — the difference between an operator finding the cause and hunting for a bug in
their agent. Failing it is what releases the lease and stops the renewals.

An hour is deliberately generous: a human-in-the-loop agent waiting on an actual human may
legitimately park for a long time, and being wrong that way costs a failed run the client can
resubmit, where being wrong the other way costs unbounded growth. Switching it off is right for
genuinely open-ended conversations with trusted clients; on a public address it means anyone who
can submit a run can leave one parked forever.

Distinct from `sync_timeout`, which bounds how long a *request* waits rather than how long a *run*
may be parked. A `sync` call against a run that parks returns after `sync_timeout` with the run
still `awaiting`; this is what eventually ends the run itself.

### When a replica is full

Every run is a spawned task, and by default nothing caps how many there are — a busy enough
server accumulates them until memory runs out, with no way to shed load on the way there. Set a
ceiling and it refuses instead:

```rust
AcpServer::builder()
    .agent(my_agent)
    .max_concurrent_runs(64)    // unset by default, which is unbounded
```

Over the ceiling, `POST /runs` answers **429 with a `Retry-After`** — a load an operator can see
and a client can wait out, rather than a crash. It pairs with the client's
[retry policy](#riding-out-a-transient-failure): 429 is one of the statuses it backs off on.

**A run parked `awaiting` a client answer does not count.** It is a suspended future waiting on a
human who may never come back, and holding capacity for it would let idle conversations starve
work that is ready to run. The slot is given up when the run parks and taken back when the answer
arrives — unchecked, so a burst of answers can briefly exceed the ceiling. That is deliberate: the
ceiling bounds what a replica *takes on*, and stranding a conversation mid-sentence to defend a
number would be the wrong trade.

Recovery replacements are admitted over the ceiling for the same reason. Refusing one would not
defer the work, it would lose the run — recovery has nobody to retry it, unlike a client meeting
a 429.

This is not a rate limit. Requests per second is a tower middleware concern; this is how many
agent invocations are alive at once, which only the server can know. With the `metrics` feature,
`acp_runs_executing` and `acp_runs_rejected_total` are what you tune the number against.

### Telling a load balancer where to send work

`GET /ping` is ACP's health check and answers "this process is up" — which is what a supervisor
deciding whether to *restart* wants. A load balancer deciding whether to *route* is asking a
different question, and answering it with liveness means a replica whose store is unreachable
keeps taking traffic and failing everything it is handed.

`GET /ready` answers that one. **Not part of ACP** — an extension, like the session resource
endpoints and the SSE variant of the event log:

```sh
curl -s localhost:8000/ready
# {"ready":true,"accepting":true,"executing":3}

# Draining, or the store is gone — 503, and:
# {"ready":false,"accepting":false,"executing":2,"reason":"draining"}
```

Unready means one of two things: this replica is [draining](#when-a-replica-is-deployed-over), or
its store cannot be reached. Both are cases where a run started here would fail, and neither is a
reason to restart the process — which is exactly why the two signals are separate.

**Being [at capacity](#when-a-replica-is-full) is deliberately not unready.** A full replica is
healthy and empties as its runs finish. Reporting it unready would pull it out of rotation under
load, pushing its share onto replicas that are also full, which report unready in turn — until a
busy fleet has removed itself from service. A 429 sheds one request; an unready replica sheds all
of them.

The store check is cached for a second, so a probe schedule does not become load on the store —
most sharply when the store is the thing already struggling. A drain is a local flag and is
reported immediately, since it costs the store nothing to know.

If you put auth in front of the server, this needs exempting along with `/ping`. A 401 on `/ready`
is indistinguishable from "do not send me traffic", so the whole fleet quietly leaves rotation
while every process stays perfectly healthy — see [Authentication](#authentication).

### When a replica is deployed over

Everything above is about a replica that *dies*. A replica being deployed over is the same
situation with one difference — it knows it is going away, and can act on that. Treating a
rolling deploy as a crash would fail every run in flight and take a lease TTL to admit it.

```rust
let (server, router) = AcpServer::builder().agent(my_agent).build()?.into_shared_router();

// On SIGTERM:
server.stop_accepting();                  // POST /runs answers 503 + Retry-After
                                          // ...and GET /ready starts answering 503,
                                          // so the balancer stops routing here...
let abandoned = server.drain(Duration::from_secs(60)).await;
```

The two steps are separate because a deployment wants to stop taking work, tell its load
balancer, and *then* wait — and the waiting is the long part. `shutdown(deadline)` does both if
there is nothing to do in between.

Draining refuses **new runs only**. Reads, cancellations, and resuming a run that is already
`awaiting` all keep working: an `awaiting` run belongs to a client that is about to answer, and
rejecting the answer would strand a run this replica is still holding. A draining replica also
stops adopting abandoned runs it comes across, since reaping one means doing that work *here*.

A run still going at the deadline is reported and has its **lease released** — not failed. This
replica is leaving and is in no position to judge a run that might have been a second from
finishing. Releasing hands the decision to whoever picks it up: a `recoverable` agent gets a
replacement started at once, anything else is failed by the next replica to read it. Both already
happen when a lease lapses; releasing is what makes them happen *now* rather than up to
`lease_ttl` later.

**A run parked `awaiting` a client is not waited for at all.** It is a suspended future waiting on
someone who may never answer, so a drain that counted it would sit out its whole deadline for a
conversation doing nothing. It is handed back immediately instead, and reported separately:

```rust
let drained = server.shutdown(DEFAULT_DRAIN_DEADLINE).await;
drained.unfinished   // ran out of deadline — consider a longer one
drained.parked       // clients were mid-conversation
```

**A drain waits for the run, not for the agent.** When it returns, every run it waited for has had
its output appended to its session and its outcome recorded — so draining and then reading the
store, which is the only way to use this, shows finished runs as finished. The distinction is not
academic: a run's outcome is written *after* its agent body returns, and a drain released at the
earlier moment reported runs as unfinished that had in fact completed.

Be clear about what that costs. **A parked conversation cannot survive its replica.** An agent that
paused to ask a question is suspended part-way through its own function, and that position lives in
this process — no other replica can resume from it. A `recoverable` agent gets a replacement
started from its input, which re-asks the question; anything else is failed. What the drain buys is
that this happens promptly and legibly, not that the conversation lives.

A replacement started this way does **not** spend a recovery attempt. The ceiling exists to stop a
run that poisons whatever executes it, and a run whose replica walked away deliberately has
demonstrated nothing of the sort — without the distinction, a rolling deploy across three replicas
would exhaust the default budget in three hops and fail the run for something the agent did not do.
A replacement's own record starts clean, so one that then dies for real is charged normally.

Sequencing with axum's own graceful shutdown matters. `stop_accepting` first, so requests arriving
during the drain are refused rather than started; axum's shutdown second, to stop accepting
connections; `drain` last, since the run tasks are not tied to the connections that started them.
`cargo run --example graceful_shutdown` runs that sequence across two replicas, and carries tests
that fail on the wrong order rather than leaving it as advice.

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
(subscription liveness on return, and atomic session appends). `InMemoryStore`,
`RedisStore` and `PostgresStore` are all implemented against exactly that contract, and
the multi-replica test suite runs unchanged against each of them — which is the check
that the contract is real rather than a description of whichever backend came first.

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
| `GET /runs/{run_id}/events` | ✅ | `list_run_events`, `attach`, `attach_after` |
| `GET /session/{session_id}` | ✅ | `get_session`, `fetch_session_history` |

Plus resource endpoints backing the links ACP puts in a session — `GET /session/{id}/messages/{i}`
for history and `GET /session/{id}/state` for state — and, with the `well-known` feature,
`GET /.well-known/agent.yml` for open discovery. None of those are in the OpenAPI document: the
spec says history and state are URLs on resource servers, and leaves where they point to the
implementation.

`GET /ready` is an extension too — a readiness signal for a load balancer, distinct from ACP's
`/ping` liveness check. See [Telling a load balancer where to send work](#telling-a-load-balancer-where-to-send-work).

`GET /runs/{run_id}/events` also answers `Accept: text/event-stream` with a resumable SSE stream
honouring `Last-Event-ID`. That is an extension too — the OpenAPI document describes only the
JSON list, which remains what a client gets by default.

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
cargo run --example graceful_shutdown   # taking one replica out without killing its work

cargo run --example authenticated_server --features well-known   # bearer token, both halves
```

Each example's header comment carries the equivalent `curl` invocations.

## Benchmarks

```sh
cargo bench                       # in-memory only, no services needed
ACP_TEST_REDIS_URL=redis://127.0.0.1:6379 \
ACP_TEST_POSTGRES_URL=postgres://postgres@127.0.0.1:5432/acp_test \
  cargo bench --all-features      # adds the networked backends
```

Three suites: `serialization` (the wire format, paid on every path), `store` (each backend
operation by operation), and `run` (whole runs over real HTTP). A configured-but-unreachable
backend is *skipped* here, unlike in the tests — a benchmark that cannot connect has nothing to
report, whereas a test that cannot connect is silently testing nothing.

Deliberately **not** a CI gate. Shared runners are noisy enough to produce false failures more
often than real signal; the value is a local baseline to run before and after a change.

Indicative figures from one developer machine — treat the *ratios* as the finding, not the
absolute numbers:

| Operation | in-memory | Redis | Postgres |
| --- | --- | --- | --- |
| `append_event` | 2.2 µs | 318 µs | 1.26 ms |
| `publish`, no subscriber | 168 ns | 162 µs | 296 µs |
| `publish`, one subscriber | 194 ns | 205 µs | 397 µs |

That gap is the thing worth knowing. A token-by-token agent hits `append_event` and `publish`
once per token, so on a shared store a thousand-token response is a thousand network
round-trips — which is why emitting is `async` and returns `Result` at all, and why the choice
between backends is a throughput decision rather than only a durability one.

Two other numbers that shape the API:

- **A whole `sync` run of an agent that returns immediately: ~320 µs**, over loopback HTTP with
  the in-memory store. That is the framework's own floor — routing, serialization, the run
  snapshot and the event log.
- **Reading a session grows linearly with its length**: ~153 µs at one turn, ~606 µs at 200.
  An agent is handed its history on every turn, which is precisely the cost
  `load_state`/`store_state` exists to let it avoid.

## Tests

```sh
cargo test --all-features
```

163 tests: wire-format round-trips for every schema, end-to-end coverage of discovery, all three
run modes, streaming order and aggregation, await/resume, cancellation of both running and
awaiting runs, session continuity and the error paths — plus a multi-replica suite that starts
two servers sharing one store and drives a run through one while observing, resuming and
cancelling it through the other — including killing a replica's whole runtime mid-run and
asserting the run gets reaped rather than hanging, and that a replayable one is replaced
by a fresh linked run.

Four suites deliberately avoid racing what they test, since the gaps they cover are
microseconds wide and would otherwise pass by luck: `ordering.rs`, `resumption.rs`,
`reaping.rs` and `cancellation_handoff.rs` each wrap the store in a decorator that makes the
window wide and fixed, so a violation fails every time rather than occasionally.

That approach earned its keep here: adding the Postgres backend, where every write is a
network round-trip rather than a memory write, surfaced three ordering bugs that every
backend had and neither of the fast ones ever exposed.

The multi-replica suite runs against **all three** backends. The Redis and Postgres halves are
skipped unless their URLs are set; when one *is* set, an unreachable backend fails the run
rather than quietly skipping — a suite that silently tests nothing is worse than one that is
honestly absent:

```sh
ACP_TEST_REDIS_URL=redis://127.0.0.1:6379 \
ACP_TEST_POSTGRES_URL=postgres://postgres@127.0.0.1:5432/acp_test \
  cargo test --all-features
```

CI runs the suite on stable, beta and the 1.86 MSRV against real Redis and Postgres services, plus
`rustfmt`, `clippy -D warnings`, each feature combination built alone, a nightly `cargo doc`
with `-D warnings`, and `cargo package`.

## Logging

Every run executes inside a `tracing` span carrying the run id, agent name, replica id and
session id. That matters because an agent's own output comes from inside `agent.run` — without
the span it interleaves with every other concurrent run and cannot be told apart afterwards.
With it, anything the agent logs is attributable for free:

```sh
RUST_LOG=info cargo run --example echo_server
```

```text
INFO acp.run{run_id=0195e2a1-… agent=echo replica=agent-host-7}: my_agent: calling the model
```

Abandoning and recovering a run gets its own `acp.reap` span, opened only by the replica that
*wins* the claim — so an abandoned run produces one span however many replicas noticed it.

There are deliberately **no per-request spans**. A run outlives the request that created it and
can be resumed or cancelled through a different request on a different replica, so a request
span could never cover one. Requests are `tower-http`'s `TraceLayer`, layered on the router like
any other middleware — the crate does not duplicate it.

### Metrics

With the `metrics` feature, the server records through the [`metrics`] facade. It records but
does not export — whichever exporter you install receives them, and installing none costs an
atomic load per call. Same bargain as the router: the crate does not pick your stack.

| Metric | Type | Labels |
| --- | --- | --- |
| `acp_runs_total` | counter | `agent`, `status` |
| `acp_run_duration_seconds` | histogram | `agent`, `status` |
| `acp_runs_in_flight` | gauge | `agent` |
| `acp_runs_executing` | gauge | — |
| `acp_runs_rejected_total` | counter | — |
| `acp_lease_renew_failures_total` | counter | — |
| `acp_runs_reaped_total` | counter | `agent` |
| `acp_recovery_claims_total` | counter | `outcome` (`won`/`lost`) |
| `acp_recoveries_started_total` | counter | `agent` |
| `acp_recovery_exhausted_total` | counter | `agent` |

`acp_runs_executing` and `acp_runs_rejected_total` are what you set
[`max_concurrent_runs`](#when-a-replica-is-full) against. Neither is labelled by agent, and
that is deliberate: a submission is refused before the agent is looked up, so the name need only
be *syntactically* valid — it need not be one this server hosts. Labelling it would let anyone
mint unbounded time series by submitting fresh names.

The lease and recovery counters are the ones worth a dashboard. Individually those events are
already logged; what a log cannot answer is *"is this happening more than it used to"*, which is
the question that matters when a fleet starts losing replicas.

Store latency is opt-in, because wrapping the store you passed in would mean `server.store()`
handing back something else:

```rust
use rusty_acp::server::store::MeteredStore;

let store = MeteredStore::new(Arc::new(RedisStore::connect("redis://127.0.0.1/").await?));
```

That adds `acp_store_operation_duration_seconds` and `acp_store_failures_total`, labelled by
operation — `put_run`, `append_event`, and so on.

**No metric is labelled by run id**, and none should be: that is one time series per run, which
degrades a metrics backend slowly enough that nobody connects it to the change that caused it.
Run ids belong on spans, which is where they are. A test asserts this rather than a comment.

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
[`metrics`]: https://docs.rs/metrics
