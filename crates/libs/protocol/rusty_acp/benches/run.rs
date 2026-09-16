//! What the framework costs to run an agent that does nothing.
//!
//! Everything here goes through the real HTTP surface — a client calling a
//! server over a loopback socket — so the numbers include serialization, axum's
//! routing, the store writes and the event log. That is deliberate. Measuring
//! the executor in isolation would produce a smaller, prettier number that no
//! deployment can observe.
//!
//! The agents do nothing on purpose. What is left after subtracting an agent
//! that returns immediately *is* the framework's overhead, which is the
//! question a baseline exists to answer.

#![cfg(all(feature = "server", feature = "client"))]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rusty_acp::client::AcpClient;
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Message, RunCreateRequest, SessionId};
use tokio::runtime::Runtime;

/// How many parts the streaming agent emits per run.
const TOKENS: usize = 100;

fn runtime() -> Runtime {
    Runtime::new().expect("a tokio runtime")
}

/// A server with the agents these benchmarks drive, on an ephemeral port.
async fn start_server() -> AcpClient {
    // Returns immediately: a run of this measures the framework, not the agent.
    let noop = agent_fn(
        AgentManifest::new(AgentName::new("noop").unwrap(), "Returns immediately"),
        |_ctx: RunContext| async move { Ok(()) },
    );

    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
        |ctx: RunContext| async move {
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    // The emit path, once per token — the hot path for a streaming agent, and
    // the place the store choice shows up most sharply.
    let streamer = agent_fn(
        AgentManifest::new(AgentName::new("streamer").unwrap(), "Emits a fixed number of parts"),
        |ctx: RunContext| async move {
            let mut writer = ctx.begin_message().await?;
            for index in 0..TOKENS {
                writer.push_text(format!("token-{index} ")).await?;
            }
            writer.finish().await?;
            Ok(())
        },
    );

    let router =
        AcpServer::builder().agent(noop).agent(echo).agent(streamer).build().unwrap().into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    AcpClient::new(format!("http://{addr}")).unwrap()
}

/// A whole run, end to end: create, execute, settle, respond.
fn run_sync(c: &mut Criterion) {
    let runtime = runtime();
    let client = runtime.block_on(start_server());

    let mut group = c.benchmark_group("run/sync");

    // The floor: everything the framework does, with an agent that does
    // nothing.
    group.bench_function("noop", |b| {
        b.to_async(&runtime)
            .iter(|| async { client.run_sync("noop", [Message::user("go")]).await.unwrap() });
    });

    // The same, plus one emitted message — the difference is what a single
    // reply costs.
    group.bench_function("echo", |b| {
        b.to_async(&runtime)
            .iter(|| async { client.run_sync("echo", [Message::user("hello")]).await.unwrap() });
    });

    group.finish();
}

/// A run in a session, which adds the history writes on both ends.
///
/// Compared against `run/sync/echo`, the difference is what session bookkeeping
/// costs — two appends, and the write that has to land before the run is
/// allowed to complete.
///
/// A **fresh session per iteration**, which matters more than it looks. An
/// agent is given its session's history, so the server reads the whole session
/// on every run: reusing one session would make each iteration more expensive
/// than the last, and the reported number would then depend on how many
/// iterations criterion happened to choose. `growing-session` below measures
/// that growth deliberately instead of smuggling it in here.
fn run_in_session(c: &mut Criterion) {
    let runtime = runtime();
    let client = runtime.block_on(start_server());

    let mut group = c.benchmark_group("run/sync");
    group.bench_function("echo-in-fresh-session", |b| {
        b.to_async(&runtime).iter(|| async {
            client
                .create_run(
                    RunCreateRequest::new(
                        AgentName::new("echo").unwrap(),
                        [Message::user("hello")],
                    )
                    .with_session_id(SessionId::new()),
                )
                .await
                .unwrap()
        });
    });
    group.finish();
}

/// What a *long* session costs to read back.
///
/// Measured as a read rather than as another run, and that is the whole point:
/// a run *appends*, so benchmarking runs against one session grows it during
/// the measurement and the reported number ends up depending on how many
/// iterations criterion chose rather than on the session size. Reading is
/// idempotent, so each iteration sees the same session it was given.
///
/// The cost matters because an agent is handed its session's history on every
/// turn. This is the number that argues for `load_state`/`store_state`.
fn session_history(c: &mut Criterion) {
    let runtime = runtime();
    let client = runtime.block_on(start_server());

    let mut group = c.benchmark_group("session/read");

    for turns in [1usize, 50, 200] {
        let session_id = SessionId::new();
        runtime.block_on(async {
            for _ in 0..turns {
                client
                    .create_run(
                        RunCreateRequest::new(
                            AgentName::new("echo").unwrap(),
                            [Message::user("a previous turn")],
                        )
                        .with_session_id(session_id),
                    )
                    .await
                    .unwrap();
            }
        });

        // Two messages per turn — one in, one out.
        group.throughput(Throughput::Elements(turns as u64 * 2));
        group.bench_with_input(BenchmarkId::from_parameter(turns), &turns, |b, _| {
            b.to_async(&runtime).iter(|| async { client.get_session(session_id).await.unwrap() });
        });
    }

    group.finish();
}

/// A streaming run, measured per token.
///
/// Throughput is set to the token count, so the report is the per-emit cost
/// rather than a number that only means anything against this exact `TOKENS`.
fn streaming(c: &mut Criterion) {
    let runtime = runtime();
    let client = runtime.block_on(start_server());

    let mut group = c.benchmark_group("run/streaming");
    group.throughput(Throughput::Elements(TOKENS as u64));

    // Consumed as a stream: the fan-out path, with a subscriber attached.
    group.bench_with_input(BenchmarkId::new("consumed", TOKENS), &TOKENS, |b, _| {
        b.to_async(&runtime).iter(|| async {
            use futures_util::StreamExt;
            let mut stream = client.stream("streamer", [Message::user("go")]).await.unwrap();
            let mut seen = 0usize;
            while let Some(event) = stream.next().await {
                event.unwrap();
                seen += 1;
            }
            seen
        });
    });

    // The same agent run without anyone streaming it, so the events are
    // appended and published to nobody. The gap against `consumed` is what
    // fan-out costs.
    group.bench_with_input(BenchmarkId::new("unwatched", TOKENS), &TOKENS, |b, _| {
        b.to_async(&runtime)
            .iter(|| async { client.run_sync("streamer", [Message::user("go")]).await.unwrap() });
    });

    group.finish();
}

/// Discovery, which serves manifests from memory and is the cheapest thing the
/// server does — worth a number so that stays true.
fn discovery(c: &mut Criterion) {
    let runtime = runtime();
    let client = runtime.block_on(start_server());

    let mut group = c.benchmark_group("discovery");
    group.bench_function("list_agents", |b| {
        b.to_async(&runtime).iter(|| async { client.list_all_agents().await.unwrap() });
    });
    group.finish();
}

criterion_group!(benches, run_sync, run_in_session, session_history, streaming, discovery);
criterion_main!(benches);
