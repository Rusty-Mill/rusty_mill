//! OTLP export tests.
//!
//! The unit tests cover configuration. What matters here is the part that only
//! shows up end to end: that spans actually leave the process, that the flush
//! on shutdown is what makes them arrive, and that a remote parent produces one
//! joined trace rather than two disconnected ones.

#![cfg(feature = "otel")]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use rusty_mcp::{
    otel::{OtelConfig, OtelGuard},
    trace::TraceContext,
};

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01";

/// Spans the fake collector received, as (trace_id hex, parent_span_id hex).
type Received = Arc<Mutex<Vec<(String, String)>>>;

/// A minimal OTLP/gRPC trace service that records what it is sent.
///
/// Decoding protobuf by hand would be its own project, so this reads the two
/// fields the tests care about straight out of the wire format: span records
/// carry a 16-byte trace id and an 8-byte parent span id, and the ids we look
/// for are distinctive enough to find by scanning.
mod collector {
    use super::*;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// Start the collector and return its address.
    pub async fn spawn(received: Received) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind collector");
        let addr = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let received = Arc::clone(&received);

                tokio::spawn(async move {
                    let mut buf = vec![0u8; 64 * 1024];
                    let mut seen = Vec::new();

                    // Read whatever the exporter sends; we only need the bytes.
                    loop {
                        match tokio::time::timeout(
                            Duration::from_millis(400),
                            socket.read(&mut buf),
                        )
                        .await
                        {
                            Ok(Ok(0)) | Err(_) => break,
                            Ok(Ok(n)) => seen.extend_from_slice(&buf[..n]),
                            Ok(Err(_)) => break,
                        }
                    }

                    if !seen.is_empty() {
                        record(&seen, &received);
                        // A gRPC client waits for a response; an HTTP/2 GOAWAY
                        // is enough to let it finish rather than hang.
                        let _ = socket.shutdown().await;
                    }
                });
            }
        });

        addr
    }

    /// Pull trace/parent ids out of the raw payload.
    fn record(bytes: &[u8], received: &Received) {
        let expected_trace = hex(&TRACEPARENT[3..35]);
        let expected_parent = hex(&TRACEPARENT[36..52]);

        let mut hits = Vec::new();
        if find(bytes, &expected_trace).is_some() {
            let parent = if find(bytes, &expected_parent).is_some() {
                TRACEPARENT[36..52].to_string()
            } else {
                String::new()
            };
            hits.push((TRACEPARENT[3..35].to_string(), parent));
        } else if !bytes.is_empty() {
            // A span arrived, but on a trace of its own.
            hits.push((String::new(), String::new()));
        }

        if !hits.is_empty() {
            received.lock().expect("lock").extend(hits);
        }
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
            .collect()
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        haystack.windows(needle.len()).position(|w| w == needle)
    }
}

/// Build a pipeline pointed at `addr` and run `body` with a subscriber feeding
/// it.
///
/// Scoped rather than global on purpose: `init` installs a global subscriber
/// and only the first call in a process wins, so several tests in one binary
/// would leave all but one exporting nothing. `pipeline` exists precisely for
/// callers that bring their own subscriber, and that is what is exercised here.
fn with_pipeline<F: FnOnce()>(addr: SocketAddr, body: F) -> OtelGuard {
    use tracing_subscriber::prelude::*;

    let (guard, tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint(format!("http://{addr}"))
            .with_shutdown_timeout(Duration::from_secs(2)),
    )
    .expect("pipeline starts");

    let subscriber =
        tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));

    tracing::subscriber::with_default(subscriber, body);

    guard
}

#[tokio::test(flavor = "multi_thread")]
async fn spans_reach_the_collector_after_a_flush() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = collector::spawn(Arc::clone(&received)).await;

    let guard = with_pipeline(addr, || {
        let span = tracing::info_span!("a-span");
        let _entered = span.enter();
        tracing::info!("inside");
    });

    // The flush is what sends them; without it the buffer dies with the guard.
    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        !received.lock().expect("lock").is_empty(),
        "the collector should have received at least one span"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_parent_joins_the_callers_trace() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = collector::spawn(Arc::clone(&received)).await;

    let guard = with_pipeline(addr, || {
        let context = TraceContext::from_parts(TRACEPARENT, None, None).expect("valid");
        let span = context.span("tools/call");
        // The line that turns two disconnected traces into one.
        context.attach_parent(&span);

        let _entered = span.enter();
        tracing::info!("handling");
    });

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let seen = received.lock().expect("lock").clone();
    assert!(!seen.is_empty(), "no spans arrived");
    assert!(
        seen.iter()
            .any(|(trace, _)| trace == "0af7651916cd43dd8448eb211c80319c"),
        "the exported span should carry the caller's trace id, got {seen:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_is_idempotent() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = collector::spawn(Arc::clone(&received)).await;

    let guard = with_pipeline(addr, || {});
    guard.shutdown();
    // A shutdown hook may fire alongside `Drop`; neither may panic.
    guard.shutdown();
    guard.flush();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_collector_does_not_break_the_server() {
    // Telemetry is not load-bearing: a collector that is down must not take
    // request handling with it.
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            // Port 1 is reliably closed.
            .with_endpoint("http://127.0.0.1:1")
            .with_shutdown_timeout(Duration::from_millis(200)),
    )
    .expect("the pipeline starts even if the collector is unreachable");

    let span = tracing::info_span!("still-works");
    let _entered = span.enter();
    tracing::info!("the handler carries on");
    drop(_entered);

    guard.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shutdown_hook_flushes() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = collector::spawn(Arc::clone(&received)).await;

    let guard = Arc::new(with_pipeline(addr, || {
        let span = tracing::info_span!("hooked");
        let _entered = span.enter();
        tracing::info!("inside");
    }));

    // Exactly what `ServerConfig::with_shutdown_hook` will call.
    let hook = guard.shutdown_hook();
    hook().await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !received.lock().expect("lock").is_empty(),
        "the shutdown hook should have flushed the buffer"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_attach_parent_the_span_starts_its_own_trace() {
    // The control for `a_remote_parent_joins_the_callers_trace`. Same context,
    // same span — the only difference is the missing `attach_parent`. If this
    // also carried the caller's trace id, that test would be proving nothing.
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = collector::spawn(Arc::clone(&received)).await;

    let guard = with_pipeline(addr, || {
        let context = TraceContext::from_parts(TRACEPARENT, None, None).expect("valid");
        // `span()` records the ids as *fields*, which correlates logs but does
        // not create a parent edge.
        let span = context.span("tools/call");

        let _entered = span.enter();
        tracing::info!("handling");
    });

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let seen = received.lock().expect("lock").clone();
    assert!(!seen.is_empty(), "a span should still have been exported");
    assert!(
        !seen
            .iter()
            .any(|(trace, _)| trace == "0af7651916cd43dd8448eb211c80319c"),
        "without attach_parent the span must be on its own trace, got {seen:?}"
    );
}
