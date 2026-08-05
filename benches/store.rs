//! What each storage backend costs, operation by operation.
//!
//! The point is the *gap*, not the absolute numbers. `InMemoryStore` is the
//! floor — it is a lock and a `Vec` — so the ratio between it and a networked
//! backend is what a deployment is actually choosing when it picks one. A
//! streaming agent hits `append_event` and `publish` once per token, which is
//! where that ratio stops being academic.
//!
//! In-memory always runs. The networked backends run only when configured, the
//! same way the tests gate them:
//!
//! ```sh
//! ACP_TEST_REDIS_URL=redis://127.0.0.1:6379 \
//! ACP_TEST_POSTGRES_URL=postgres://postgres@127.0.0.1:5432/acp_test \
//!   cargo bench --bench store
//! ```
//!
//! Unlike the test suite, a configured-but-unreachable backend is skipped with
//! a warning rather than failing. A benchmark that cannot connect has nothing
//! to report; a test that cannot connect is silently testing nothing, which is
//! why the two differ.

#![cfg(feature = "server")]

use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rusty_acp::server::store::{InMemoryStore, Notification, Store};
use rusty_acp::types::{AgentName, Event, Message, MessagePart, Run, SessionId};
use tokio::runtime::Runtime;

fn runtime() -> Runtime {
    Runtime::new().expect("a tokio runtime")
}

/// Every backend available in this environment, named for the report.
///
/// `mut` is only used by the feature-gated pushes below, so a build with
/// neither networked backend enabled sees a binding that is never mutated.
#[allow(unused_mut)]
fn backends(runtime: &Runtime) -> Vec<(&'static str, Arc<dyn Store>)> {
    let mut backends: Vec<(&'static str, Arc<dyn Store>)> =
        vec![("in-memory", Arc::new(InMemoryStore::default()))];

    #[cfg(feature = "redis-store")]
    if let Ok(url) = std::env::var("ACP_TEST_REDIS_URL") {
        use rusty_acp::server::store::{RedisStore, RedisStoreConfig};
        let config = RedisStoreConfig {
            key_prefix: format!("acp-bench:{}", uuid::Uuid::new_v4()),
            ttl: Some(Duration::from_secs(300)),
        };
        match runtime.block_on(RedisStore::connect_with(&url, config)) {
            Ok(store) => backends.push(("redis", Arc::new(store))),
            Err(error) => eprintln!("skipping redis: {error}"),
        }
    }

    #[cfg(feature = "postgres-store")]
    if let Ok(url) = std::env::var("ACP_TEST_POSTGRES_URL") {
        use rusty_acp::server::store::{PostgresStore, PostgresStoreConfig};
        let config = PostgresStoreConfig {
            table_prefix: format!("acp_bench_{}", uuid::Uuid::new_v4().simple()),
            ..PostgresStoreConfig::default()
        };
        match runtime.block_on(PostgresStore::connect_with(&url, config)) {
            Ok(store) => backends.push(("postgres", Arc::new(store))),
            Err(error) => eprintln!("skipping postgres: {error}"),
        }
    }

    let _ = runtime;
    backends
}

/// Run `body` against every available backend, then drop the stores *inside*
/// the runtime.
///
/// The drop location matters. A `PostgresStore` holds a pool, and sqlx returns
/// a connection to its pool by spawning onto the current runtime — so dropping
/// one with no runtime context panics inside a destructor, which Rust cannot
/// unwind out of and turns into an abort.
fn for_each_backend(runtime: &Runtime, mut body: impl FnMut(&'static str, &Arc<dyn Store>)) {
    let backends = backends(runtime);
    for (name, store) in &backends {
        body(name, store);
    }
    runtime.block_on(async move { drop(backends) });
}

/// A run already present in `store`, ready to be appended to.
fn seeded_run(runtime: &Runtime, store: &Arc<dyn Store>) -> Run {
    let run = Run::new(AgentName::new("bench").unwrap(), None);
    runtime.block_on(store.put_run(&run)).expect("seed the run");
    run
}

/// Appending an event: the streaming hot path, once per token.
fn append_event(c: &mut Criterion) {
    let runtime = runtime();
    let mut group = c.benchmark_group("store/append_event");

    for_each_backend(&runtime, |name, store| {
        let run = seeded_run(&runtime, store);
        let event = Event::MessagePart { part: MessagePart::text("a plausible token") };

        group.bench_with_input(BenchmarkId::from_parameter(name), store, |b, store| {
            b.to_async(&runtime)
                .iter(|| async { store.append_event(run.run_id, &event).await.unwrap() });
        });
    });

    group.finish();
}

/// Publishing with nobody listening — the common case for `async` mode, where
/// the fan-out cost is zero and only the round-trip remains.
fn publish_unsubscribed(c: &mut Criterion) {
    let runtime = runtime();
    let mut group = c.benchmark_group("store/publish/no-subscriber");

    for_each_backend(&runtime, |name, store| {
        let run = seeded_run(&runtime, store);

        group.bench_with_input(BenchmarkId::from_parameter(name), store, |b, store| {
            b.to_async(&runtime).iter(|| async {
                let event = Event::MessagePart { part: MessagePart::text("token") };
                store.publish(run.run_id, Notification::event_at(0, event)).await.unwrap()
            });
        });
    });

    group.finish();
}

/// Publishing with a subscriber attached, which is what a streaming client
/// costs. The difference against the case above is the fan-out.
fn publish_subscribed(c: &mut Criterion) {
    let runtime = runtime();
    let mut group = c.benchmark_group("store/publish/one-subscriber");

    for_each_backend(&runtime, |name, store| {
        let run = seeded_run(&runtime, store);
        let subscription = runtime.block_on(store.subscribe(run.run_id)).expect("subscribe");

        group.bench_with_input(BenchmarkId::from_parameter(name), store, |b, store| {
            b.to_async(&runtime).iter(|| async {
                let event = Event::MessagePart { part: MessagePart::text("token") };
                store.publish(run.run_id, Notification::event_at(0, event)).await.unwrap()
            });
        });

        // Inside the runtime: a Postgres subscription is a dedicated
        // connection, returned to its pool by a spawn on drop.
        runtime.block_on(async move { drop(subscription) });
    });

    group.finish();
}

/// Writing a run snapshot, which happens on every state transition.
fn put_run(c: &mut Criterion) {
    let runtime = runtime();
    let mut group = c.benchmark_group("store/put_run");

    for_each_backend(&runtime, |name, store| {
        let run = seeded_run(&runtime, store);

        group.bench_with_input(BenchmarkId::from_parameter(name), store, |b, store| {
            b.to_async(&runtime).iter(|| async { store.put_run(&run).await.unwrap() });
        });
    });

    group.finish();
}

/// Reading a run, which every request does — including the reap check.
fn get_run(c: &mut Criterion) {
    let runtime = runtime();
    let mut group = c.benchmark_group("store/get_run");

    for_each_backend(&runtime, |name, store| {
        let run = seeded_run(&runtime, store);

        group.bench_with_input(BenchmarkId::from_parameter(name), store, |b, store| {
            b.to_async(&runtime).iter(|| async { store.get_run(run.run_id).await.unwrap() });
        });
    });

    group.finish();
}

/// Reading the tail of an event log — what a resuming stream pays per
/// reconnection, and the reason `events_from` seeks rather than filters.
fn events_from(c: &mut Criterion) {
    const LOG_LENGTH: u64 = 1000;

    let runtime = runtime();
    let mut group = c.benchmark_group("store/events_from");

    for_each_backend(&runtime, |name, store| {
        let run = seeded_run(&runtime, store);
        runtime.block_on(async {
            for index in 0..LOG_LENGTH {
                let event = Event::generic(serde_json::json!({ "n": index }));
                store.append_event(run.run_id, &event).await.unwrap();
            }
        });

        // From near the end: the shape of an actual reconnection, and the case
        // that would be linear in the whole log if the backend filtered.
        group.bench_with_input(BenchmarkId::new("tail", name), store, |b, store| {
            b.to_async(&runtime)
                .iter(|| async { store.events_from(run.run_id, LOG_LENGTH - 10).await.unwrap() });
        });

        // From the start: a fresh stream on an existing run, and the honest
        // worst case.
        group.bench_with_input(BenchmarkId::new("whole", name), store, |b, store| {
            b.to_async(&runtime).iter(|| async { store.events_from(run.run_id, 0).await.unwrap() });
        });
    });

    group.finish();
}

/// Appending to a session, which must be atomic against other replicas and so
/// is the operation most likely to differ between backends.
fn append_session_messages(c: &mut Criterion) {
    let runtime = runtime();
    let mut group = c.benchmark_group("store/append_session_messages");

    for_each_backend(&runtime, |name, store| {
        let session_id = SessionId::new();

        group.bench_with_input(BenchmarkId::from_parameter(name), store, |b, store| {
            b.to_async(&runtime).iter(|| async {
                store
                    .append_session_messages(
                        session_id,
                        "http://acp.example",
                        vec![Message::user("a turn in a conversation")],
                    )
                    .await
                    .unwrap()
            });
        });
    });

    group.finish();
}

/// Renewing a lease, which every executing replica does three times per lease
/// lifetime for every run it is running.
fn renew_lease(c: &mut Criterion) {
    let runtime = runtime();
    let mut group = c.benchmark_group("store/renew_lease");

    for_each_backend(&runtime, |name, store| {
        let run = seeded_run(&runtime, store);

        group.bench_with_input(BenchmarkId::from_parameter(name), store, |b, store| {
            b.to_async(&runtime).iter(|| async {
                store
                    .renew_lease(run.run_id, "bench-replica", Duration::from_secs(30))
                    .await
                    .unwrap()
            });
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    append_event,
    publish_unsubscribed,
    publish_subscribed,
    put_run,
    get_run,
    events_from,
    append_session_messages,
    renew_lease,
);
criterion_main!(benches);
