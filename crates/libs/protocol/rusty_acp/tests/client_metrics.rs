//! What the *client* records, and what it says while it is deciding.
//!
//! The client's job in these paths is to not surface something to the caller:
//! it retries with backoff, honours `Retry-After`, reconnects a dropped stream,
//! and tolerates transient failures while polling. Every one of those is
//! invisible by design, which is why it needs counting — a caller timing
//! `run_sync` otherwise cannot tell a slow agent from a fast one behind four
//! retries.
//!
//! Asserted against a real recorder rather than on the call sites being
//! present, for the reason `metrics.rs` gives: a counter that is never
//! incremented looks exactly like a quiet system.

#![cfg(all(feature = "metrics", feature = "client"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use metrics_util::CompositeKey;
use rusty_acp::client::{AcpClient, RetryPolicy};
use rusty_acp::types::Message;

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

fn total(recorded: &[Recorded], name: &str) -> u64 {
    find(recorded, name)
        .iter()
        .map(|entry| match entry.value {
            DebugValue::Counter(value) => value,
            ref other => panic!("expected a counter, got {other:?}"),
        })
        .sum()
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

/// A current-thread runtime built *inside* the recorder scope.
///
/// `with_local_recorder` installs a thread-local, so anything recorded from
/// another thread lands nowhere. Driving the futures on this thread is what
/// keeps the two together — the same shape `metrics.rs` uses, and the reason
/// these are `#[test]` rather than `#[tokio::test]`.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

/// A server that answers the first `fail_first` requests with 503.
///
/// Raw axum rather than an `AcpServer`: the point is to make the *client*
/// retry, and a real ACP server has no way to be told to fail twice and then
/// stop.
async fn flaky_server(fail_first: usize) -> String {
    use axum::http::StatusCode;
    use axum::routing::post;

    let remaining = Arc::new(AtomicUsize::new(fail_first));
    let app = axum::Router::new().route(
        "/runs",
        post(move || {
            let remaining = Arc::clone(&remaining);
            async move {
                if remaining.load(Ordering::SeqCst) > 0 {
                    remaining.fetch_sub(1, Ordering::SeqCst);
                    return (StatusCode::SERVICE_UNAVAILABLE, String::new());
                }
                let run = serde_json::json!({
                    "run_id": "00000000-0000-4000-8000-000000000000",
                    "agent_name": "echo",
                    "status": "completed",
                    "created_at": "2020-01-01T00:00:00Z",
                    "output": [],
                });
                (StatusCode::OK, run.to_string())
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// Retries that take microseconds, so a test is not paced by its own backoff.
fn quick_retries(base_url: &str) -> AcpClient {
    AcpClient::builder(base_url)
        .retry(RetryPolicy {
            max_retries: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            // The submission is the request under test, and it is not retried
            // by default — replaying one risks a second run.
            retry_run_submission: true,
            ..RetryPolicy::default()
        })
        .build()
        .unwrap()
}

/// Every attempt is counted, so the gap between this and the server's own
/// request count is exactly what the retry policy added.
#[test]
fn attempts_are_counted_including_the_retried_ones() {
    with_recorder(|snapshotter| {
        let recorded = runtime().block_on(async {
            let base_url = flaky_server(2).await;
            let client = quick_retries(&base_url);
            client.run_sync("echo", [Message::user("hi")]).await.unwrap();
            snapshot(snapshotter)
        });

        assert_eq!(
            total(&recorded, "acp_client_requests_total"),
            3,
            "two failures and a success is three attempts"
        );
        assert_eq!(total(&recorded, "acp_client_retries_total"), 2);
    });
}

/// The retry counter says *why*, drawn from a fixed set rather than from the
/// error text — one series per error string would be the cardinality mistake
/// this crate keeps warning about.
#[test]
fn a_retry_records_a_bounded_reason() {
    with_recorder(|snapshotter| {
        let recorded = runtime().block_on(async {
            let base_url = flaky_server(1).await;
            let client = quick_retries(&base_url);
            client.run_sync("echo", [Message::user("hi")]).await.unwrap();
            snapshot(snapshotter)
        });

        let retries = find(&recorded, "acp_client_retries_total");
        assert_eq!(retries.len(), 1);
        let reason = retries[0].label("reason").expect("a retry says why");
        assert!(
            ["status", "transport", "retry_after"].contains(&reason),
            "unexpected reason label {reason:?}"
        );
    });
}

/// Exhausting the policy is counted apart from the retries themselves.
///
/// The two answer different questions — "are we retrying" and "is retrying
/// helping" — and only the second is worth an alert.
#[test]
fn exhausting_the_policy_is_counted_separately() {
    with_recorder(|snapshotter| {
        let recorded = runtime().block_on(async {
            // Never succeeds, so every attempt is spent.
            let base_url = flaky_server(usize::MAX).await;
            let client = quick_retries(&base_url);
            let _ = client.run_sync("echo", [Message::user("hi")]).await;
            snapshot(snapshotter)
        });

        assert_eq!(total(&recorded, "acp_client_retries_exhausted_total"), 1);
        assert_eq!(
            total(&recorded, "acp_client_requests_total"),
            4,
            "three retries on top of the first attempt"
        );
    });
}

/// Backoff time is recorded at sub-second resolution.
///
/// The figure that explains latency looking like a slow agent when it is not —
/// and the common backoff is milliseconds, so a seconds-only counter would
/// round almost all of it to nothing.
#[test]
fn time_spent_in_backoff_is_recorded() {
    with_recorder(|snapshotter| {
        let recorded = runtime().block_on(async {
            let base_url = flaky_server(2).await;
            let client = quick_retries(&base_url);
            client.run_sync("echo", [Message::user("hi")]).await.unwrap();
            snapshot(snapshotter)
        });

        let histogram = find(&recorded, "acp_client_backoff_duration_seconds");
        assert_eq!(histogram.len(), 1, "backoff was not recorded at sub-second resolution");
        match &histogram[0].value {
            DebugValue::Histogram(samples) => assert_eq!(samples.len(), 2, "one sample per retry"),
            other => panic!("expected a histogram, got {other:?}"),
        }
    });
}

/// No run id reaches a label. One time series per run sinks a backend slowly
/// enough that nobody connects it to the change that caused it.
#[test]
fn no_client_metric_carries_a_run_id() {
    with_recorder(|snapshotter| {
        let recorded = runtime().block_on(async {
            let base_url = flaky_server(1).await;
            let client = quick_retries(&base_url);
            client.run_sync("echo", [Message::user("hi")]).await.unwrap();
            snapshot(snapshotter)
        });

        for entry in &recorded {
            for (key, value) in &entry.labels {
                assert!(
                    !key.contains("run"),
                    "{} carries a run-shaped label {key}={value}",
                    entry.name
                );
                assert!(
                    uuid::Uuid::parse_str(value).is_err(),
                    "{} carries a uuid as the label {key}",
                    entry.name
                );
            }
        }
    });
}

/// A client that never retries records the attempt and nothing else, so the
/// counters cannot imply a policy that is switched off.
#[test]
fn a_disabled_policy_records_no_retries() {
    with_recorder(|snapshotter| {
        let recorded = runtime().block_on(async {
            let base_url = flaky_server(1).await;
            let client =
                AcpClient::builder(&base_url).retry(RetryPolicy::disabled()).build().unwrap();
            let _ = client.run_sync("echo", [Message::user("hi")]).await;
            snapshot(snapshotter)
        });

        assert_eq!(total(&recorded, "acp_client_requests_total"), 1);
        assert_eq!(total(&recorded, "acp_client_retries_total"), 0);
        assert_eq!(total(&recorded, "acp_client_retries_exhausted_total"), 0);
    });
}
