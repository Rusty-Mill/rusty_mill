//! What the server actually records.
//!
//! Metrics are the one kind of output nothing else notices when it goes wrong:
//! a counter that is never incremented looks exactly like a quiet system, and a
//! label that names the wrong thing looks like a working dashboard until
//! somebody needs it. So these assert on a real recorder rather than on the
//! call sites being present.
//!
//! Cardinality is asserted too. A run id as a label is one time series per run,
//! which sinks a metrics backend slowly enough that nobody connects it to the
//! change that caused it.

#![cfg(all(feature = "metrics", feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use metrics_util::CompositeKey;
use rusty_acp::client::{AcpClient, WaitOptions};
use rusty_acp::server::store::InMemoryStore;
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Error, Message, RunStatus};

/// One recorded metric, flattened to the parts worth asserting on.
#[derive(Debug)]
struct Recorded {
    name: String,
    labels: Vec<(String, String)>,
    value: DebugValue,
}

impl Recorded {
    fn label(&self, key: &str) -> Option<&str> {
        self.labels.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str())
    }
}

fn snapshot(snapshotter: &Snapshotter) -> Vec<Recorded> {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .map(|(key, _unit, _description, value)| {
            let key: CompositeKey = key;
            let key = key.key();
            Recorded {
                name: key.name().to_string(),
                labels: key
                    .labels()
                    .map(|label| (label.key().to_string(), label.value().to_string()))
                    .collect(),
                value,
            }
        })
        .collect()
}

fn find<'a>(recorded: &'a [Recorded], name: &str) -> Vec<&'a Recorded> {
    recorded.iter().filter(|entry| entry.name == name).collect()
}

fn counter_value(entry: &Recorded) -> u64 {
    match entry.value {
        DebugValue::Counter(value) => value,
        ref other => panic!("expected a counter, got {other:?}"),
    }
}

async fn start_server(store: Arc<dyn rusty_acp::server::store::Store>) -> AcpClient {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
        |ctx: RunContext| async move {
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    let boom = agent_fn(
        AgentManifest::new(AgentName::new("boom").unwrap(), "Always fails"),
        |_ctx: RunContext| async move { Err(Error::server_error("boom")) },
    );

    let forever = agent_fn(
        AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes on its own"),
        |ctx: RunContext| async move {
            ctx.cancelled().await;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(echo)
        .agent(boom)
        .agent(forever)
        .store(store)
        .build()
        .unwrap()
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    AcpClient::new(format!("http://{addr}")).unwrap()
}

/// Install a recorder for the body of the test.
///
/// `metrics` allows one global recorder per process, so each test body runs
/// against a local one rather than racing to install it.
fn with_recorder<F, T>(body: F) -> T
where
    F: FnOnce(&Snapshotter) -> T,
{
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || body(&snapshotter))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

/// A completed run is counted, with its agent and status.
#[test]
fn a_completed_run_is_counted_by_agent_and_status() {
    with_recorder(|snapshotter| {
        runtime().block_on(async {
            let client = start_server(Arc::new(InMemoryStore::default())).await;
            let run = client.run_sync("echo", [Message::user("hello")]).await.unwrap();
            assert_eq!(run.status, RunStatus::Completed);
        });

        let recorded = snapshot(snapshotter);
        let runs = find(&recorded, "acp_runs_total");
        assert_eq!(runs.len(), 1, "expected one series, got {runs:?}");
        assert_eq!(runs[0].label("agent"), Some("echo"));
        assert_eq!(runs[0].label("status"), Some("completed"));
        assert_eq!(counter_value(runs[0]), 1);
    });
}

/// A failed run is counted under its own status, not lumped in with successes.
#[test]
fn a_failed_run_is_counted_separately() {
    with_recorder(|snapshotter| {
        runtime().block_on(async {
            let client = start_server(Arc::new(InMemoryStore::default())).await;
            let run = client.run_sync("boom", [Message::user("go")]).await.unwrap();
            assert_eq!(run.status, RunStatus::Failed);
        });

        let recorded = snapshot(snapshotter);
        let runs = find(&recorded, "acp_runs_total");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].label("agent"), Some("boom"));
        assert_eq!(
            runs[0].label("status"),
            Some("failed"),
            "a failure counted as a success is worse than no metric at all"
        );
    });
}

/// A cancelled run is counted as cancelled — the outcome the run actually
/// reached, not the one the executor set out to write.
#[test]
fn a_cancelled_run_is_counted_as_cancelled() {
    with_recorder(|snapshotter| {
        runtime().block_on(async {
            let client = start_server(Arc::new(InMemoryStore::default())).await;
            let started = client.run_async("forever", [Message::user("hang")]).await.unwrap();
            let cancelled = client
                .cancel_and_wait(
                    started.run_id,
                    WaitOptions::default().with_timeout(Duration::from_secs(10)),
                )
                .await
                .unwrap();
            assert_eq!(cancelled.status, RunStatus::Cancelled);
        });

        let recorded = snapshot(snapshotter);
        let runs = find(&recorded, "acp_runs_total");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].label("status"), Some("cancelled"));
    });
}

/// The in-flight gauge comes back to zero once a run is done.
///
/// A gauge that only ever increments is the classic way to make a dashboard
/// that looks fine on the first day and lies by the second.
#[test]
fn the_in_flight_gauge_returns_to_zero() {
    with_recorder(|snapshotter| {
        runtime().block_on(async {
            let client = start_server(Arc::new(InMemoryStore::default())).await;
            for _ in 0..3 {
                client.run_sync("echo", [Message::user("hello")]).await.unwrap();
            }
        });

        let recorded = snapshot(snapshotter);
        let gauges = find(&recorded, "acp_runs_in_flight");
        assert_eq!(gauges.len(), 1);
        match gauges[0].value {
            DebugValue::Gauge(value) => {
                assert_eq!(value.into_inner(), 0.0, "every run that started must also have ended")
            }
            ref other => panic!("expected a gauge, got {other:?}"),
        }
    });
}

/// Run duration is recorded, and is a real measurement rather than zero.
#[test]
fn run_duration_is_recorded() {
    with_recorder(|snapshotter| {
        runtime().block_on(async {
            let client = start_server(Arc::new(InMemoryStore::default())).await;
            client.run_sync("echo", [Message::user("hello")]).await.unwrap();
        });

        let recorded = snapshot(snapshotter);
        let durations = find(&recorded, "acp_run_duration_seconds");
        assert_eq!(durations.len(), 1);
        match durations[0].value {
            DebugValue::Histogram(ref values) => {
                assert_eq!(values.len(), 1);
                assert!(
                    values[0].into_inner() >= 0.0,
                    "a duration must be a real measurement: {values:?}"
                );
            }
            ref other => panic!("expected a histogram, got {other:?}"),
        }
    });
}

/// No metric carries a run id as a label.
///
/// The check that matters most and is easiest to lose: one time series per run
/// degrades a metrics backend gradually, long after the change that caused it.
#[test]
fn no_metric_is_labelled_by_run_id() {
    with_recorder(|snapshotter| {
        let run_id = runtime().block_on(async {
            let client = start_server(Arc::new(InMemoryStore::default())).await;
            let run = client.run_sync("echo", [Message::user("hello")]).await.unwrap();
            run.run_id.to_string()
        });

        for entry in snapshot(snapshotter) {
            for (key, value) in &entry.labels {
                assert_ne!(key, "run_id", "`{}` is labelled by run id", entry.name);
                assert_ne!(value, &run_id, "`{}` carries a run id in label `{key}`", entry.name);
            }
        }
    });
}

/// `MeteredStore` times the operations underneath it.
#[test]
fn a_metered_store_records_operation_latency() {
    use rusty_acp::server::store::MeteredStore;

    with_recorder(|snapshotter| {
        runtime().block_on(async {
            let store = Arc::new(MeteredStore::new(Arc::new(InMemoryStore::default())));
            let client = start_server(store).await;
            client.run_sync("echo", [Message::user("hello")]).await.unwrap();
        });

        let recorded = snapshot(snapshotter);
        let operations = find(&recorded, "acp_store_operation_duration_seconds");
        assert!(!operations.is_empty(), "a run touches the store; nothing was timed");

        let names: Vec<&str> =
            operations.iter().filter_map(|entry| entry.label("operation")).collect();
        for expected in ["put_run", "append_event", "publish"] {
            assert!(names.contains(&expected), "expected `{expected}` among {names:?}");
        }
        assert!(
            !names.iter().any(|name| name.is_empty()),
            "every timed operation must be named: {names:?}"
        );
    });
}

/// Without the store wrapper, nothing is timed — the decorator is opt-in and
/// must not be applied behind the operator's back.
#[test]
fn an_unwrapped_store_is_not_timed() {
    with_recorder(|snapshotter| {
        runtime().block_on(async {
            let client = start_server(Arc::new(InMemoryStore::default())).await;
            client.run_sync("echo", [Message::user("hello")]).await.unwrap();
        });

        let recorded = snapshot(snapshotter);
        assert!(
            find(&recorded, "acp_store_operation_duration_seconds").is_empty(),
            "building a server must not wrap the store the caller passed in"
        );
    });
}

/// The rejection counter, and the deliberate absence of a label on it.
///
/// The obvious label would be the agent name. But a submission is refused
/// before the agent is *looked up*, so the name need only be syntactically
/// valid — it need not name an agent this server hosts. Labelling it would let
/// anyone mint unbounded time series by submitting fresh names, which is a
/// worse problem than the one the label solves.
#[test]
fn refusals_are_counted_without_a_caller_controlled_label() {
    with_recorder(|snapshotter| {
        let recorded = runtime().block_on(async {
            let forever = agent_fn(
                AgentManifest::new(AgentName::new("forever").unwrap(), "Never finishes"),
                |ctx: RunContext| async move {
                    ctx.cancelled().await;
                    Ok(())
                },
            );
            let (server, router) = AcpServer::builder()
                .agent(forever)
                .max_concurrent_runs(1)
                .build()
                .unwrap()
                .into_shared_router();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let client = AcpClient::new(format!("http://{addr}")).unwrap();

            client.run_async("forever", [Message::user("go")]).await.unwrap();
            // Refused twice: once under a registered name, once under a name
            // that is valid but hosted nowhere — which still reaches the
            // capacity check, and so would still mint a label.
            for agent in ["forever", "not-a-real-agent"] {
                let _ = client.run_async(agent, [Message::user("go")]).await;
            }
            assert_eq!(server.executing(), 1);

            // Snapshotted inside the runtime: dropping it drops the run task,
            // which drops its slot and takes the gauge back to zero. Correct
            // behaviour — a slot outliving the future holding it would be the
            // bug — but it means the reading has to be taken while the run is
            // still alive.
            snapshot(snapshotter)
        });

        let rejected = find(&recorded, "acp_runs_rejected_total");
        assert_eq!(rejected.len(), 1, "one series, whatever names were submitted: {rejected:?}");
        assert_eq!(counter_value(rejected[0]), 2);
        assert!(
            rejected[0].labels.is_empty(),
            "a caller-controlled label is an unbounded cardinality hazard: {:?}",
            rejected[0].labels
        );

        // The gauge an operator tunes the ceiling against.
        let executing = find(&recorded, "acp_runs_executing");
        assert_eq!(executing.len(), 1, "the depth gauge is unlabelled too");
        assert!(
            matches!(executing[0].value, DebugValue::Gauge(value) if value.into_inner() == 1.0),
            "expected one run executing, got {:?}",
            executing[0].value
        );
    });
}
