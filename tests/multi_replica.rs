//! Two servers, one store: the multi-replica behaviour ACP's high-availability
//! guide calls for.
//!
//! Each test starts two independent [`AcpServer`]s on separate ports sharing a
//! single [`Store`], then drives a run through **one** replica and observes or
//! controls it through the **other**. That is the property that lets replicas
//! sit behind a load balancer with no session affinity.
//!
//! Every case runs twice: once against [`InMemoryStore`] and once against
//! [`RedisStore`]. The Redis cases are skipped when no Redis is configured —
//! see [`redis_store`].

#![cfg(all(feature = "client", feature = "server"))]

use std::{sync::Arc, time::Duration};

use futures_util::StreamExt;
use rusty_acp::{
    client::{collect_run, AcpClient},
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

    let forever = agent_fn(
        AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes on its own"),
        |ctx: RunContext| async move {
            ctx.emit_generic(json!({ "phase": "started" })).await?;
            ctx.cancelled().await;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(echo)
        .agent(greeter)
        .agent(historian)
        .agent(forever)
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

/// Two replicas sharing one store.
async fn replicas(store: Arc<dyn Store>) -> (AcpClient, AcpClient) {
    let a = replica(Arc::clone(&store)).await;
    let b = replica(store).await;
    assert_ne!(a.base_url(), b.base_url(), "replicas must be distinct servers");
    (a, b)
}

fn memory_store() -> Arc<dyn Store> {
    Arc::new(InMemoryStore::default())
}

/// A Redis-backed store, or `None` when Redis is not configured.
///
/// Set `ACP_TEST_REDIS_URL` to run these. When it is set the connection *must*
/// succeed — a misconfigured CI job fails loudly rather than quietly skipping
/// the backend it was meant to exercise.
#[cfg(feature = "redis-store")]
async fn redis_store() -> Option<Arc<dyn Store>> {
    use rusty_acp::server::store::{RedisStore, RedisStoreConfig};

    let url = std::env::var("ACP_TEST_REDIS_URL").ok()?;
    // A fresh prefix per store keeps concurrent tests from colliding.
    let config = RedisStoreConfig {
        key_prefix: format!("acp-test:{}", uuid::Uuid::new_v4()),
        ttl: Some(Duration::from_secs(60)),
    };
    let store = RedisStore::connect_with(&url, config)
        .await
        .expect("ACP_TEST_REDIS_URL is set but Redis is unreachable");
    Some(Arc::new(store))
}

// ---------------------------------------------------------------------------
// The behaviours under test, each written once and run against every backend.
// ---------------------------------------------------------------------------

/// A run started on one replica is readable from the other.
async fn run_is_visible_from_the_other_replica(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;

    let run = a.run_sync("echo", [Message::user("hello")]).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let seen_by_b = b.get_run(run.run_id).await.unwrap();
    assert_eq!(seen_by_b.run_id, run.run_id);
    assert_eq!(seen_by_b.status, RunStatus::Completed);
    assert_eq!(seen_by_b.output_text(), "hello");
}

/// The event log written by one replica is served by the other, in order.
async fn event_log_is_readable_from_the_other_replica(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;

    let run = a.run_sync("echo", [Message::user("hello")]).await.unwrap();

    let from_a = a.list_run_events(run.run_id).await.unwrap();
    let from_b = b.list_run_events(run.run_id).await.unwrap();
    assert_eq!(from_a, from_b, "both replicas must serve the same log");

    let types: Vec<_> = from_b.iter().map(|event| event.event_type()).collect();
    assert_eq!(types.first(), Some(&"run.created"));
    assert_eq!(types.last(), Some(&"run.completed"));
}

/// An agent awaiting on one replica is resumed through the other.
async fn resume_routes_to_the_executing_replica(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;

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
async fn resume_streams_events_across_replicas(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;

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
async fn cancel_routes_to_the_executing_replica(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;

    let started = a.run_async("forever", [Message::user("hang")]).await.unwrap();

    // The agent is parked inside A; the cancellation arrives at B.
    let cancelled = b.cancel_and_wait(started.run_id).await.unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert!(cancelled.finished_at.is_some());

    assert_eq!(a.get_run(started.run_id).await.unwrap().status, RunStatus::Cancelled);
}

/// Cancelling an awaiting run works across replicas too.
async fn cancel_reaches_an_awaiting_run_across_replicas(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;

    let paused = a.run_sync("greeter", [Message::user("hi")]).await.unwrap();
    assert_eq!(paused.status, RunStatus::Awaiting);

    let cancelled = b.cancel_and_wait(paused.run_id).await.unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
}

/// A session written by one replica is served, and extended, by the other.
async fn sessions_are_shared_between_replicas(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;
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
async fn agents_see_history_written_by_the_other_replica(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;
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
async fn streamed_output_is_persisted_for_other_replicas(store: Arc<dyn Store>) {
    let (a, b) = replicas(store).await;

    let stream = a.stream("echo", [Message::user("streamed")]).await.unwrap();
    let run = collect_run(stream).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let seen_by_b = b.get_run(run.run_id).await.unwrap();
    assert_eq!(seen_by_b.output_text(), "streamed");
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
                    super::$name(memory_store()).await;
                }
            )+
        }

        #[cfg(feature = "redis-store")]
        mod redis {
            use super::*;
            $(
                #[tokio::test]
                async fn $name() {
                    let Some(store) = redis_store().await else {
                        eprintln!(
                            "skipping {}: set ACP_TEST_REDIS_URL to run the Redis backend tests",
                            stringify!($name)
                        );
                        return;
                    };
                    super::$name(store).await;
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
);
