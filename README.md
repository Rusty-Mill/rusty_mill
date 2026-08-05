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

Both directions speak the same protocol, so a Rust agent is a drop-in peer for a Python (BeeAI),
TypeScript, LangChain or CrewAI one.

## Install

```toml
[dependencies]
rusty-acp = "0.1"
```

Default features are `client` + `server`. Take just what you need:

```toml
rusty-acp = { version = "0.1", default-features = false }                      # types only
rusty-acp = { version = "0.1", default-features = false, features = ["client"] }
rusty-acp = { version = "0.1", default-features = false, features = ["server"] }
```

Minimum supported Rust version is **1.86**, verified in CI on every change.

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
        ctx.reply_text(ctx.input_text());
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
        ctx.reply_text(ctx.input_text());
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
let finished = client.wait_for_run(started.run_id, None).await?;
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
let mut writer = ctx.begin_message();
for token in tokens {
    writer.push_text(token);
}
writer.finish();
```

### Pausing for client input (await / resume)

An agent can stop mid-run to ask a question. The run moves to `awaiting` with an
`await_request`; the client answers with `POST /runs/{run_id}`.

```rust
// Agent side
let resume = ctx.await_json(serde_json::json!({ "question": "What is your name?" })).await?;
let name = resume.as_value()["answer"].as_str().unwrap_or("stranger");
ctx.reply_text(format!("Hello, {name}!"));
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

`POST /runs/{run_id}/cancel` returns `202` and moves the run to `cancelling`. The agent's future
is dropped, and long-running agents can react promptly by selecting on `ctx.cancelled()`.

```rust
let cancelled = client.cancel_and_wait(run_id).await?;   // polls until it leaves `cancelling`
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

## Examples

```sh
cargo run --example echo_server      # two agents: one plain, one streaming
cargo run --example awaiting_agent   # pauses mid-run to ask a question
cargo run --example client_demo      # drives every client operation against a running server
```

Each example's header comment carries the equivalent `curl` invocations.

## Tests

```sh
cargo test --all-features
```

48 tests: wire-format round-trips for every schema, and end-to-end coverage of discovery, all
three run modes, streaming order and aggregation, await/resume, cancellation of both running and
awaiting runs, session continuity, and the error paths.

CI runs the suite on stable, beta and the 1.86 MSRV, plus `rustfmt`, `clippy -D warnings`, each
of the four feature combinations built alone, a nightly `cargo doc` with `-D warnings`, and
`cargo package`.

## Notes on the server

- Runs are held in memory, capped by `AcpServerBuilder::max_runs` (default 1024). Active runs are
  never evicted; the oldest terminal ones go first.
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
