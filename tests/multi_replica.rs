//! Two servers, one store: the multi-replica behaviour ACP's high-availability
//! guide calls for.
//!
//! Each test starts two independent [`AcpServer`]s on separate ports sharing a
//! single [`Store`], then drives a run through **one** replica and observes or
//! controls it through the **other**. That is the property that lets replicas
//! sit behind a load balancer with no session affinity.
//!
//! Every case runs three times: against [`InMemoryStore`], [`RedisStore`] and
//! `PostgresStore`. The Redis and Postgres cases are skipped when those
//! backends are not configured — see [`redis_backend`] and [`postgres_backend`].

#![cfg(all(feature = "client", feature = "server"))]

use std::{sync::Arc, time::Duration};

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use rusty_acp::{
    client::{collect_run, AcpClient, WaitOptions},
    server::{agent_fn, store::InMemoryStore, store::Store, AcpServer, RunContext},
    types::{
        AgentManifest, AgentName, AwaitRequest, Message, RunCreateRequest, RunMode,
        RunResumeRequest, RunStatus, SessionId,
    },
};
use serde_json::json;

/// Start one replica backed by `store`, returning a client pointed at it.
async fn replica(store: Arc<dyn Store>) -> AcpClient {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
        |ctx: RunContext| async move {
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    let greeter = agent_fn(
        AgentManifest::new(AgentName::new("greeter").unwrap(), "Asks for a name, then greets"),
        |ctx: RunContext| async move {
            let resume =
                ctx.await_request(AwaitRequest::new(json!({ "question": "name?" }))).await?;
            let name = resume.as_value()["answer"].as_str().unwrap_or("stranger").to_string();
            let mut writer = ctx.begin_message().await?;
            writer.push_text(format!("Hello, {name}")).await?;
            writer.push_text("!").await?;
            writer.finish().await?;
            Ok(())
        },
    );

    let historian = agent_fn(
        AgentManifest::new(
            AgentName::new("historian").unwrap(),
            "Reports what history it was given",
        ),
        |ctx: RunContext| async move {
            let seen: Vec<String> = ctx.history().iter().map(|message| message.text()).collect();
            ctx.reply_text(seen.join(",")).await?;
            Ok(())
        },
    );

    // Uses session state rather than history to remember: the point is that
    // state written on one replica is readable on another.
    let counter = agent_fn(
        AgentManifest::new(AgentName::new("counter").unwrap(), "Counts turns in session state"),
        |ctx: RunContext| async move {
            let previous: u32 = ctx.load_state().await?.unwrap_or(0);
            let next = previous + 1;
            ctx.store_state(&next).await?;
            ctx.reply_text(format!("turn {next}")).await?;
            Ok(())
        },
    );

    let forever = agent_fn(
        AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes on its own"),
        |ctx: RunContext| async move {
            ctx.emit_generic(json!({ "phase": "started" })).await?;
            ctx.cancelled().await;
            Ok(())
        },
    );

    // Every replica hosts the same agents, which is the premise of the whole
    // deployment shape — and a hard requirement for recovery, since the replica
    // that reaps an abandoned run is the one that has to re-run it.
    let hangs = agent_fn(
        AgentManifest::new(
            AgentName::new("hangs").unwrap(),
            "Replayable, but never finishes on its own",
        ),
        |ctx: RunContext| async move {
            ctx.cancelled().await;
            Ok(())
        },
    )
    .with_recovery();

    let router = AcpServer::builder()
        .agent(echo)
        .agent(greeter)
        .agent(historian)
        .agent(counter)
        .agent(forever)
        .agent(hangs)
        .store(store)
        // Pinning the base URL is what a load balancer address would do: history
        // links must not depend on which replica served the request.
        .base_url("http://acp.example")
        .build()
        .expect("server builds")
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    AcpClient::new(format!("http://{addr}")).unwrap()
}

/// A replica running on its own Tokio runtime, on its own thread, so a test can
/// destroy it the way a process death would.
///
/// Aborting the serving task is not enough: the run executor and its lease
/// renewal are separate spawned tasks and would keep renewing, so the run would
/// never look abandoned. Dropping the whole runtime takes every task with it,
/// which is exactly what losing the process does.
struct KillableReplica {
    shutdown: tokio::sync::oneshot::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl KillableReplica {
    /// Destroy the replica and wait until its runtime is really gone.
    ///
    /// Joining the thread makes this deterministic: once it returns, nothing
    /// belonging to that replica can renew a lease again.
    fn kill(self) {
        let _ = self.shutdown.send(());
        let _ = self.thread.join();
    }
}

/// Start a replica whose lease lapses quickly, on a runtime a test can drop.
fn short_lease_replica(backend: &Backend) -> (AcpClient, KillableReplica) {
    short_lease_replica_with(backend, 3)
}

/// As [`short_lease_replica`], with an explicit recovery attempt budget.
fn short_lease_replica_with(
    backend: &Backend,
    max_recovery_attempts: u32,
) -> (AcpClient, KillableReplica) {
    let backend = backend.clone();
    let (addr_tx, addr_rx) = std::sync::mpsc::channel();
    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();

    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("replica runtime builds");

        runtime.block_on(async move {
            // Connected *inside* this runtime, so destroying the runtime takes this
            // replica's own connections with it and leaves the survivor's alone —
            // which is what losing a process does.
            let store = backend.connect().await;

            let forever = agent_fn(
                AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes on its own"),
                |ctx: RunContext| async move {
                    ctx.emit_generic(json!({ "phase": "started" })).await?;
                    ctx.cancelled().await;
                    Ok(())
                },
            );

            // Replayable, and never finishes on its own — so a run of it is
            // reliably still in flight when its replica is destroyed, and its
            // replacement is reliably still running afterwards.
            let hangs = agent_fn(
                AgentManifest::new(
                    AgentName::new("hangs").unwrap(),
                    "Replayable, but never finishes on its own",
                ),
                |ctx: RunContext| async move {
                    ctx.cancelled().await;
                    Ok(())
                },
            )
            .with_recovery();

            let router = AcpServer::builder()
                .agent(forever)
                .agent(hangs)
                .store(store)
                .base_url("http://acp.example")
                // Short enough to observe in a test; the mechanism is identical
                // at the 30s default.
                .lease_ttl(Duration::from_secs(1))
                .max_recovery_attempts(max_recovery_attempts)
                .build()
                .expect("server builds")
                .into_router();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            addr_tx.send(listener.local_addr().unwrap()).unwrap();
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        // Dropping the runtime here aborts the run executor and its lease
        // renewal along with everything else this replica had in flight.
    });

    let addr = addr_rx.recv().expect("replica reports its address");
    (AcpClient::new(format!("http://{addr}")).unwrap(), KillableReplica { shutdown, thread })
}

/// Two replicas sharing one store.
async fn replicas(backend: &Backend) -> (AcpClient, AcpClient) {
    let a = replica(backend.connect().await).await;
    let b = replica(backend.connect().await).await;
    assert_ne!(a.base_url(), b.base_url(), "replicas must be distinct servers");
    (a, b)
}

/// Where a test's replicas get their storage.
///
/// Deliberately a *factory* rather than one shared handle. The replicas this
/// suite models are separate processes, and for a pooled backend that is not a
/// detail: a test that destroys a replica's runtime would otherwise take
/// connections the surviving replica still depends on, and the survivor would
/// stall on a pool that thinks they are alive. Each replica connects for
/// itself, exactly as another process would.
///
/// `InMemoryStore` is the exception, and not really one: it has no external
/// storage to connect to, so sharing the handle *is* sharing the storage.
#[derive(Clone)]
enum Backend {
    Memory(Arc<dyn Store>),
    #[cfg(feature = "redis-store")]
    Redis {
        url: String,
        key_prefix: String,
    },
    #[cfg(feature = "postgres-store")]
    Postgres {
        url: String,
        table_prefix: String,
    },
}

impl Backend {
    /// A store handle for one replica.
    async fn connect(&self) -> Arc<dyn Store> {
        match self {
            Backend::Memory(store) => Arc::clone(store),
            #[cfg(feature = "redis-store")]
            Backend::Redis { url, key_prefix } => {
                use rusty_acp::server::store::{RedisStore, RedisStoreConfig};
                let config = RedisStoreConfig {
                    key_prefix: key_prefix.clone(),
                    ttl: Some(Duration::from_secs(60)),
                };
                Arc::new(
                    RedisStore::connect_with(url, config)
                        .await
                        .expect("ACP_TEST_REDIS_URL is set but Redis is unreachable"),
                )
            }
            #[cfg(feature = "postgres-store")]
            Backend::Postgres { url, table_prefix } => {
                use rusty_acp::server::store::{PostgresStore, PostgresStoreConfig};
                let config = PostgresStoreConfig {
                    table_prefix: table_prefix.clone(),
                    // A replica needs few connections and the suite runs many
                    // replicas in one process; the default 10 each is sized for
                    // a deployment, not for seventeen of them at once.
                    max_connections: 4,
                    ..PostgresStoreConfig::default()
                };
                Arc::new(
                    PostgresStore::connect_with(url, config)
                        .await
                        .expect("ACP_TEST_POSTGRES_URL is set but Postgres is unreachable"),
                )
            }
        }
    }
}

fn memory_backend() -> Backend {
    Backend::Memory(Arc::new(InMemoryStore::default()))
}

/// The Redis backend, or `None` when Redis is not configured.
///
/// Set `ACP_TEST_REDIS_URL` to run these. When it is set the connection *must*
/// succeed — a misconfigured CI job fails loudly rather than quietly skipping
/// the backend it was meant to exercise.
#[cfg(feature = "redis-store")]
fn redis_backend() -> Option<Backend> {
    let url = std::env::var("ACP_TEST_REDIS_URL").ok()?;
    // A fresh prefix per test keeps concurrent tests from colliding, while the
    // replicas within one test share it — that shared prefix is what makes them
    // one deployment.
    Some(Backend::Redis { url, key_prefix: format!("acp-test:{}", uuid::Uuid::new_v4()) })
}

/// The Postgres backend, or `None` when Postgres is not configured.
///
/// Set `ACP_TEST_POSTGRES_URL` to run these. As with Redis, a URL that is set
/// but unreachable fails rather than skipping — a backend that quietly tests
/// nothing is worse than one that is honestly absent.
#[cfg(feature = "postgres-store")]
fn postgres_backend() -> Option<Backend> {
    let url = std::env::var("ACP_TEST_POSTGRES_URL").ok()?;
    // Postgres identifiers cannot start with a digit and dislike hyphens, hence
    // the shape.
    Some(Backend::Postgres {
        url,
        table_prefix: format!("acp_test_{}", uuid::Uuid::new_v4().simple()),
    })
}

// ---------------------------------------------------------------------------
// The behaviours under test, each written once and run against every backend.
// ---------------------------------------------------------------------------

/// A run started on one replica is readable from the other.
async fn run_is_visible_from_the_other_replica(backend: Backend) {
    let (a, b) = replicas(&backend).await;

    let run = a.run_sync("echo", [Message::user("hello")]).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let seen_by_b = b.get_run(run.run_id).await.unwrap();
    assert_eq!(seen_by_b.run_id, run.run_id);
    assert_eq!(seen_by_b.status, RunStatus::Completed);
    assert_eq!(seen_by_b.output_text(), "hello");
}

/// The event log written by one replica is served by the other, in order.
async fn event_log_is_readable_from_the_other_replica(backend: Backend) {
    let (a, b) = replicas(&backend).await;

    let run = a.run_sync("echo", [Message::user("hello")]).await.unwrap();

    let from_a = a.list_run_events(run.run_id).await.unwrap();
    let from_b = b.list_run_events(run.run_id).await.unwrap();
    assert_eq!(from_a, from_b, "both replicas must serve the same log");

    let types: Vec<_> = from_b.iter().map(|event| event.event_type()).collect();
    assert_eq!(types.first(), Some(&"run.created"));
    assert_eq!(types.last(), Some(&"run.completed"));
}

/// An agent awaiting on one replica is resumed through the other.
async fn resume_routes_to_the_executing_replica(backend: Backend) {
    let (a, b) = replicas(&backend).await;

    // Replica A starts the run and parks it.
    let paused = a.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);

    // Replica B — which has never seen this run — resumes it. The payload has
    // to reach the agent running inside A.
    let resumed = b
        .resume_run(RunResumeRequest::new(
            paused.run_id,
            json!({ "answer": "Ada" }).into(),
            RunMode::Sync,
        ))
        .await
        .unwrap();

    assert_eq!(resumed.status, RunStatus::Completed);
    assert_eq!(resumed.output_text(), "Hello, Ada!");

    // And A agrees, because the state lives in the store rather than in B.
    assert_eq!(a.get_run(paused.run_id).await.unwrap().output_text(), "Hello, Ada!");
}

/// Resuming through the other replica streams the events the first one emits.
///
/// This is the cross-replica streaming case: the agent runs inside replica A
/// and publishes events there, while the SSE connection is served by replica B.
async fn resume_streams_events_across_replicas(backend: Backend) {
    let (a, b) = replicas(&backend).await;

    let paused = a.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);

    let mut stream = b
        .stream_resume(RunResumeRequest::new(
            paused.run_id,
            json!({ "answer": "Grace" }).into(),
            RunMode::Stream,
        ))
        .await
        .unwrap();

    let mut parts = Vec::new();
    let mut types = Vec::new();
    while let Some(event) = stream.next().await {
        let event = event.unwrap();
        types.push(event.event_type().to_string());
        if let rusty_acp::types::Event::MessagePart { part } = &event {
            parts.push(part.content.clone().unwrap_or_default());
        }
    }

    // The parts were emitted by the agent inside A and relayed by B.
    assert_eq!(parts, ["Hello, Grace", "!"]);
    assert_eq!(types.last().map(String::as_str), Some("run.completed"));
    // Control signals must never leak onto a client's event stream.
    assert!(!types.iter().any(|t| t == "resume" || t == "cancel"));
}

/// A run executing on one replica is cancelled through the other.
async fn cancel_routes_to_the_executing_replica(backend: Backend) {
    let (a, b) = replicas(&backend).await;

    let started = a.run_async("forever", [Message::user("hang")]).await.unwrap();

    // The agent is parked inside A; the cancellation arrives at B.
    let cancelled = b.cancel_and_wait(started.run_id, WaitOptions::default()).await.unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert!(cancelled.finished_at.is_some());

    assert_eq!(a.get_run(started.run_id).await.unwrap().status, RunStatus::Cancelled);
}

/// Cancelling an awaiting run works across replicas too.
async fn cancel_reaches_an_awaiting_run_across_replicas(backend: Backend) {
    let (a, b) = replicas(&backend).await;

    let paused = a.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);

    let cancelled = b.cancel_and_wait(paused.run_id, WaitOptions::default()).await.unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
}

/// A session written by one replica is served, and extended, by the other.
async fn sessions_are_shared_between_replicas(backend: Backend) {
    let (a, b) = replicas(&backend).await;
    let session_id = SessionId::new();

    let request = |text: &str| {
        RunCreateRequest::new(AgentName::new("echo").unwrap(), [Message::user(text)])
            .with_session_id(session_id)
    };

    // First run on A, second on B — the conversation has to carry over.
    a.create_run(request("first")).await.unwrap();
    b.create_run(request("second")).await.unwrap();

    for client in [&a, &b] {
        let session = client.get_session(session_id).await.unwrap();
        assert_eq!(session.id, session_id);
        // Two runs, each contributing one input and one output message.
        assert_eq!(session.history.len(), 4, "both replicas must see the whole history");
        // History links point at the shared base URL, not at a single replica.
        assert!(session.history.iter().all(|url| url.starts_with("http://acp.example/")));
    }
}

/// An agent sees history written by a run that executed on the other replica.
///
/// Asserted through the agent itself rather than by dereferencing history URLs:
/// those point at the configured load-balancer address, which is right for a
/// real deployment but not resolvable from a test.
async fn agents_see_history_written_by_the_other_replica(backend: Backend) {
    let (a, b) = replicas(&backend).await;
    let session_id = SessionId::new();

    let request = |text: &str| {
        RunCreateRequest::new(AgentName::new("historian").unwrap(), [Message::user(text)])
            .with_session_id(session_id)
    };

    // The first run, on A, starts with nothing behind it.
    let first = a.create_run(request("first")).await.unwrap();
    assert_eq!(first.output_text(), "");

    // The second, on B, is handed everything A wrote: its input and its output.
    let second = b.create_run(request("second")).await.unwrap();
    assert_eq!(second.output_text(), "first,");
}

/// A streamed run started on one replica lands its output where the other can
/// read it.
async fn streamed_output_is_persisted_for_other_replicas(backend: Backend) {
    let (a, b) = replicas(&backend).await;

    let stream = a.stream("echo", [Message::user("streamed")]).await.unwrap();
    let run = collect_run(stream).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let seen_by_b = b.get_run(run.run_id).await.unwrap();
    assert_eq!(seen_by_b.output_text(), "streamed");
}

/// State written by an agent on one replica is readable by one on the other.
async fn session_state_crosses_replicas(backend: Backend) {
    let (a, b) = replicas(&backend).await;
    let session_id = SessionId::new();

    let request = || {
        RunCreateRequest::new(AgentName::new("counter").unwrap(), [Message::user("go")])
            .with_session_id(session_id)
    };

    // First turn on A writes state; second on B must read what A wrote.
    assert_eq!(a.create_run(request()).await.unwrap().output_text(), "turn 1");
    assert_eq!(b.create_run(request()).await.unwrap().output_text(), "turn 2");
    assert_eq!(a.create_run(request()).await.unwrap().output_text(), "turn 3");
}

/// Storing state points `Session.state` at the document, and the URL resolves.
async fn session_state_is_exposed_as_a_link(backend: Backend) {
    let (a, b) = replicas(&backend).await;
    let session_id = SessionId::new();

    a.create_run(
        RunCreateRequest::new(AgentName::new("counter").unwrap(), [Message::user("go")])
            .with_session_id(session_id),
    )
    .await
    .unwrap();

    // Both replicas report the same link, built from the shared base URL rather
    // than from whichever replica happened to serve the request.
    for client in [&a, &b] {
        let session = client.get_session(session_id).await.unwrap();
        assert_eq!(
            session.state.as_deref(),
            Some(format!("http://acp.example/session/{session_id}/state").as_str())
        );
    }

    // The state itself is not inlined into the session.
    let raw = serde_json::to_value(b.get_session(session_id).await.unwrap()).unwrap();
    assert!(raw["state"].is_string(), "state must be a link, not the document");
}

/// A run whose executing replica dies is failed rather than left hanging.
///
/// This is the failure the [sole-writer invariant][sw] exposes: with the
/// executing replica gone, nothing is left to write a terminal state, consume a
/// resume, or apply a cancel. Killing the task that serves replica A is the
/// closest a test can get to losing the process.
///
/// [sw]: https://github.com/baileyrd/rusty_acp/issues/8
async fn a_run_whose_replica_dies_is_reaped(backend: Backend) {
    let (a, replica_a) = short_lease_replica(&backend);
    let b = replica(backend.connect().await).await;

    let started = a.run_async("forever", [Message::user("hang")]).await.unwrap();
    assert!(!started.status.is_terminal());

    // Replica A stops existing, taking its lease renewal with it.
    replica_a.kill();

    // B notices the moment anyone asks: no live lease on a non-terminal run
    // means the writer is gone.
    let reaped = b
        .wait_for_run(
            started.run_id,
            WaitOptions::default()
                .poll_every(Duration::from_millis(100))
                .with_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();

    assert_eq!(reaped.status, RunStatus::Failed);
    assert!(reaped.finished_at.is_some());
    let error = reaped.clone().into_result().unwrap_err();
    assert!(
        error.message.contains("abandoned"),
        "the error should say what happened: {}",
        error.message
    );

    // And the event log ends with run.failed rather than trailing off mid-run.
    let events = b.list_run_events(started.run_id).await.unwrap();
    assert!(matches!(events.last(), Some(rusty_acp::types::Event::RunFailed { .. })));
}

/// A live run is never mistaken for an abandoned one.
///
/// The guard against an over-eager reaper: a replica that keeps renewing its
/// lease keeps its runs, however long they take.
async fn a_live_run_is_not_reaped(backend: Backend) {
    let (a, _replica_a) = short_lease_replica(&backend);
    let b = replica(backend.connect().await).await;

    let started = a.run_async("forever", [Message::user("hang")]).await.unwrap();

    // Several lease lifetimes pass with the replica alive and renewing.
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let seen = b.get_run(started.run_id).await.unwrap();
        assert!(
            !seen.status.is_terminal(),
            "a renewing replica must keep its run: {}",
            seen.status
        );
    }

    // Still cancellable, so the run is genuinely alive rather than merely unreaped.
    let cancelled = a
        .cancel_and_wait(
            started.run_id,
            WaitOptions::default().with_timeout(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
}

/// Pull the replacement run id out of an abandoned run's error, if it has one.
fn replaced_by(run: &rusty_acp::types::Run) -> Option<String> {
    run.error.as_ref()?.data.as_ref()?.get("replaced_by")?.as_str().map(str::to_string)
}

/// A recoverable run whose replica dies is replaced by a fresh, linked run.
///
/// The abandoned run keeps its own history and stays failed; the replacement
/// gets a new id and a clean log. Nothing already streamed to a client is
/// retracted, and no run ends up with two sets of output.
async fn a_recoverable_run_is_replaced_when_its_replica_dies(backend: Backend) {
    let (a, replica_a) = short_lease_replica(&backend);
    let b = replica(backend.connect().await).await;

    let started = a.run_async("hangs", [Message::user("work")]).await.unwrap();
    replica_a.kill();

    let abandoned = b
        .wait_for_run(
            started.run_id,
            WaitOptions::default()
                .poll_every(Duration::from_millis(100))
                .with_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();

    assert_eq!(abandoned.status, RunStatus::Failed);

    // The failed run points at its replacement, using the specification's own
    // slot for structured error detail.
    let replacement_id = replaced_by(&abandoned).expect("the failed run links to its replacement");
    assert_ne!(replacement_id, started.run_id.to_string());

    // The replacement is a real, running run — not just a recorded id.
    let replacement_id: rusty_acp::types::RunId = replacement_id.parse().unwrap();
    let replacement = b.get_run(replacement_id).await.unwrap();
    assert!(!replacement.status.is_terminal(), "the replacement should be running");
    assert_eq!(replacement.agent_name, abandoned.agent_name);

    // It records what it replaces, and cancelling it proves it is genuinely
    // executing rather than a phantom record.
    let events = b.list_run_events(replacement_id).await.unwrap();
    let replaces = events.iter().find_map(|event| match event {
        rusty_acp::types::Event::Generic { generic } => generic.get("replaces").cloned(),
        _ => None,
    });
    assert_eq!(
        replaces.and_then(|value| value.as_str().map(str::to_string)),
        Some(started.run_id.to_string())
    );

    let cancelled = b
        .cancel_and_wait(
            replacement_id,
            WaitOptions::default().with_timeout(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
}

/// An agent that has not opted in is never replayed.
///
/// The whole point of the default: replaying an agent with external side
/// effects would repeat them, and the server cannot tell which agents those are.
async fn a_run_that_did_not_opt_in_is_only_failed(backend: Backend) {
    let (a, replica_a) = short_lease_replica(&backend);
    let b = replica(backend.connect().await).await;

    // `forever` does not call `with_recovery`.
    let started = a.run_async("forever", [Message::user("work")]).await.unwrap();
    replica_a.kill();

    let abandoned = b
        .wait_for_run(
            started.run_id,
            WaitOptions::default()
                .poll_every(Duration::from_millis(100))
                .with_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();

    assert_eq!(abandoned.status, RunStatus::Failed);
    assert_eq!(replaced_by(&abandoned), None, "an agent that did not opt in must not be replayed");
}

/// Recovery stops once the attempt budget is spent.
///
/// With a budget of one, the first attempt is also the last: the run is failed
/// without a replacement, so a run that kills whatever executes it cannot
/// migrate around the fleet forever.
async fn recovery_stops_at_the_attempt_budget(backend: Backend) {
    // The budget has to be set on *both*: the replica that reaps an abandoned
    // run is the one that decides whether to replace it, so a fleet with
    // mismatched budgets behaves like whichever replica happened to notice.
    let (a, replica_a) = short_lease_replica_with(&backend, 1);
    let (b, _replica_b) = short_lease_replica_with(&backend, 1);

    let started = a.run_async("hangs", [Message::user("work")]).await.unwrap();
    replica_a.kill();

    let abandoned = b
        .wait_for_run(
            started.run_id,
            WaitOptions::default()
                .poll_every(Duration::from_millis(100))
                .with_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();

    assert_eq!(abandoned.status, RunStatus::Failed);
    assert_eq!(replaced_by(&abandoned), None, "the budget was spent on the first attempt");
}

/// A stream dropped on one replica is resumed on the other, without a gap.
///
/// The point of resumption in a fleet: the load balancer that routes the
/// reconnection has no reason to send it back to the replica the client was
/// talking to, and with the run's log in the shared store it does not need to.
/// The replica serving the replay here is not the one executing the run.
async fn a_dropped_stream_resumes_on_the_other_replica(backend: Backend) {
    let (a, b) = replicas(&backend).await;
    let http = reqwest::Client::new();

    // Runs on A, and pauses awaiting input — so the log is settled and finite
    // while the run is still live, with no timing to wait on.
    let paused = a.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);

    let read_stream = |base: String, last: Option<u64>| {
        let http = http.clone();
        async move {
            let mut request = http
                .get(format!("{base}/runs/{}/events", paused.run_id))
                .header("accept", "text/event-stream");
            if let Some(last) = last {
                request = request.header("last-event-id", last.to_string());
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), 200);

            let mut collected: Vec<(u64, String)> = Vec::new();
            let mut stream = response.bytes_stream().eventsource();
            while let Some(message) = stream.next().await {
                let message = message.unwrap();
                collected.push((message.id.parse().unwrap(), message.event.clone()));
                // `run.awaiting` is terminal for a stream.
                if message.event == "run.awaiting" {
                    break;
                }
            }
            collected
        }
    };

    // Read the whole log through A, then resume partway through B.
    let whole = read_stream(a.base_url().to_string(), None).await;
    assert!(whole.len() >= 3, "expected a log to resume into, got {whole:?}");

    let resume_after = whole[0].0;
    let tail = read_stream(b.base_url().to_string(), Some(resume_after)).await;

    let expected: Vec<(u64, String)> = whole.iter().skip(1).cloned().collect();
    assert_eq!(
        tail, expected,
        "the other replica must serve exactly the events after the one acknowledged"
    );
    assert_eq!(tail.last().unwrap().1, "run.awaiting");
}

// ---------------------------------------------------------------------------
// Backend bindings.
// ---------------------------------------------------------------------------

/// Generate one `#[tokio::test]` per behaviour per backend.
macro_rules! backend_tests {
    ($($name:ident),+ $(,)?) => {
        mod in_memory {
            use super::*;
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(memory_backend()).await;
                }
            )+
        }

        #[cfg(feature = "redis-store")]
        mod redis {
            use super::*;
            $(
                #[tokio::test]
                async fn $name() {
                    let Some(backend) = redis_backend() else {
                        eprintln!(
                            "skipping {}: set ACP_TEST_REDIS_URL to run the Redis backend tests",
                            stringify!($name)
                        );
                        return;
                    };
                    super::$name(backend).await;
                }
            )+
        }

        #[cfg(feature = "postgres-store")]
        mod postgres {
            use super::*;
            $(
                #[tokio::test]
                async fn $name() {
                    let Some(backend) = postgres_backend() else {
                        eprintln!(
                            "skipping {}: set ACP_TEST_POSTGRES_URL to run the Postgres backend tests",
                            stringify!($name)
                        );
                        return;
                    };
                    super::$name(backend).await;
                }
            )+
        }
    };
}

backend_tests!(
    run_is_visible_from_the_other_replica,
    event_log_is_readable_from_the_other_replica,
    resume_routes_to_the_executing_replica,
    resume_streams_events_across_replicas,
    cancel_routes_to_the_executing_replica,
    cancel_reaches_an_awaiting_run_across_replicas,
    sessions_are_shared_between_replicas,
    agents_see_history_written_by_the_other_replica,
    streamed_output_is_persisted_for_other_replicas,
    session_state_crosses_replicas,
    session_state_is_exposed_as_a_link,
    a_run_whose_replica_dies_is_reaped,
    a_live_run_is_not_reaped,
    a_recoverable_run_is_replaced_when_its_replica_dies,
    a_run_that_did_not_opt_in_is_only_failed,
    recovery_stops_at_the_attempt_budget,
    a_dropped_stream_resumes_on_the_other_replica,
);
