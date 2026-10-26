//! OTLP span export tests.
//!
//! The unit tests cover how spans are built and encoded. What matters here is
//! the part that only shows up end to end: that spans actually leave the process
//! as OTLP/JSON, that the flush on shutdown is what makes them arrive, and that
//! a remote parent produces one joined trace rather than two disconnected ones.

#![cfg(feature = "otel")]

mod otel_support;

use std::{sync::Arc, time::Duration};

use otel_support::Collector;
use rusty_json::Value;
use rusty_log::tracing_subscriber::prelude::*;
use rusty_mcp::{
    otel::{OtelConfig, OtelGuard},
    trace::TraceContext,
};

const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
const PARENT: &str = "00f067aa0ba902b7";
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01";
const UNSAMPLED: &str = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-00";

/// Build a pipeline pointed at `collector` and run `body` with a subscriber
/// feeding it.
///
/// Scoped rather than global on purpose: `init` installs a global subscriber
/// and only the first call in a process wins, so several tests in one binary
/// would leave all but one exporting nothing. `pipeline` exists precisely for
/// callers that bring their own subscriber, and that is what is exercised here.
fn with_pipeline(config: OtelConfig, body: impl FnOnce()) -> OtelGuard {
    let (guard, layer) = rusty_mcp::otel::pipeline(config).expect("pipeline starts");
    rusty_log::subscriber::with_default(
        rusty_log::tracing_subscriber::registry().with(layer),
        body,
    );
    guard
}

fn config_for(collector: &Collector) -> OtelConfig {
    OtelConfig::new("test-server")
        .with_endpoint(collector.url())
        .without_metrics()
        .with_shutdown_timeout(Duration::from_secs(2))
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn named<'a>(spans: &'a [Value], name: &str) -> &'a Value {
    spans
        .iter()
        .find(|span| string(span, "name") == Some(name))
        .unwrap_or_else(|| panic!("no span named {name} in {spans:?}"))
}

#[test]
fn spans_reach_the_collector_after_a_flush() {
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector).with_service_version("1.2.3"), || {
        let span = rusty_log::info_span!("a-span", answer = 42);
        let _entered = span.enter();
        rusty_log::info!("inside");
    });

    // The flush is what sends them; without it the buffer dies with the guard.
    guard.shutdown();

    let pushes = collector.pushes();
    assert_eq!(pushes.len(), 1, "{pushes:?}");
    assert_eq!(pushes[0].path, "/v1/traces");
    assert_eq!(pushes[0].content_type, "application/json");

    let resource = &pushes[0].body["resourceSpans"][0]["resource"]["attributes"];
    let attributes: Vec<_> = resource
        .as_array()
        .expect("attributes")
        .iter()
        .map(|a| (string(a, "key"), string(&a["value"], "stringValue")))
        .collect();
    assert!(attributes.contains(&(Some("service.name"), Some("test-server"))));
    assert!(attributes.contains(&(Some("service.version"), Some("1.2.3"))));
    assert_eq!(
        string(
            &pushes[0].body["resourceSpans"][0]["scopeSpans"][0]["scope"],
            "name"
        ),
        Some("rusty-mcp")
    );

    let spans = collector.spans();
    let span = named(&spans, "a-span");
    assert_eq!(string(&span["attributes"][0], "key"), Some("answer"));
    assert_eq!(
        string(&span["attributes"][0]["value"], "intValue"),
        Some("42")
    );
    assert_eq!(string(&span["events"][0], "name"), Some("inside"));
}

#[test]
fn a_remote_parent_joins_the_callers_trace() {
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector), || {
        let context =
            TraceContext::from_parts(TRACEPARENT, Some("vendor=opaque"), None).expect("valid");
        // `span()` records the ids as fields; the layer turns them into a parent.
        let span = context.span("tools/call");
        let _entered = span.enter();
        rusty_log::info!("handling");
    });

    guard.shutdown();

    let spans = collector.spans();
    let span = named(&spans, "tools/call");
    assert_eq!(string(span, "traceId"), Some(TRACE));
    assert_eq!(string(span, "parentSpanId"), Some(PARENT));
    assert_eq!(string(span, "traceState"), Some("vendor=opaque"));
    // The ids are the span's own now, not attributes beside it.
    let keys: Vec<_> = span["attributes"]
        .as_array()
        .expect("attributes")
        .iter()
        .filter_map(|a| string(a, "key"))
        .collect();
    assert!(
        !keys.contains(&"trace_id") && !keys.contains(&"parent_span_id"),
        "{keys:?}"
    );
}

#[test]
fn a_span_without_a_remote_parent_starts_its_own_trace() {
    // The control for the test above: no caller context, so no caller's trace.
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector), || {
        drop(rusty_log::info_span!("tools/call"));
    });

    guard.shutdown();

    let spans = collector.spans();
    let span = named(&spans, "tools/call");
    assert_ne!(string(span, "traceId"), Some(TRACE));
    assert!(span.get("parentSpanId").is_none(), "{span:?}");
}

#[test]
fn spans_in_one_request_form_a_tree_under_the_callers_span() {
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector), || {
        let context = TraceContext::from_parts(TRACEPARENT, None, None).expect("valid");
        let request = context.span("tools/call");
        let _entered = request.enter();
        let inner = rusty_log::info_span!("backend");
        drop(inner);
    });

    guard.shutdown();

    let spans = collector.spans();
    let request = named(&spans, "tools/call");
    let backend = named(&spans, "backend");
    assert_eq!(string(backend, "traceId"), Some(TRACE));
    assert_eq!(string(backend, "parentSpanId"), string(request, "spanId"));
}

#[test]
fn a_caller_that_did_not_sample_gets_no_spans() {
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector), || {
        let context = TraceContext::from_parts(UNSAMPLED, None, None).expect("valid");
        let request = context.span("tools/call");
        let _entered = request.enter();
        drop(rusty_log::info_span!("backend"));
    });

    guard.shutdown();

    assert!(collector.spans().is_empty(), "{:?}", collector.spans());
    assert!(collector.pushes().is_empty(), "nothing should even be sent");
}

#[test]
fn root_spans_are_sampled_by_the_configured_ratio() {
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector).with_sample_ratio(0.0), || {
        drop(rusty_log::info_span!("never"));
    });

    guard.shutdown();
    assert!(collector.spans().is_empty());
}

#[test]
fn a_full_batch_is_sent_without_waiting_for_a_flush() {
    let collector = Collector::start(200);

    let (guard, layer) = rusty_mcp::otel::pipeline(config_for(&collector)).expect("pipeline");
    rusty_log::subscriber::with_default(
        rusty_log::tracing_subscriber::registry().with(layer),
        || {
            for _ in 0..700 {
                drop(rusty_log::info_span!("busy"));
            }
        },
    );

    // 512 make a batch; no flush, and the interval is far off.
    assert!(
        collector.wait_until(Duration::from_secs(10), |c| !c.spans().is_empty()),
        "a full batch should be sent on its own"
    );
    guard.shutdown();
    assert_eq!(collector.spans().len(), 700, "the rest goes at shutdown");
}

#[test]
fn spans_set_after_creation_reach_the_collector() {
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector), || {
        let span = rusty_log::info_span!("mcp.request");
        rusty_mcp::otel::set_attribute(&span, "mcpGuardrails.rule", "rule-9b2e");
        rusty_mcp::otel::set_attribute(&span, "mcpGuardrails.score", 0.75);
    });

    guard.shutdown();

    let spans = collector.spans();
    let attributes = named(&spans, "mcp.request")["attributes"]
        .as_array()
        .expect("attributes")
        .clone();
    let value = |key: &str| {
        attributes
            .iter()
            .find(|a| string(a, "key") == Some(key))
            .map(|a| a["value"].clone())
    };
    assert_eq!(
        value("mcpGuardrails.rule").and_then(|v| v["stringValue"].as_str().map(str::to_owned)),
        Some("rule-9b2e".to_owned())
    );
    assert_eq!(
        value("mcpGuardrails.score").and_then(|v| v["doubleValue"].as_f64()),
        Some(0.75)
    );
}

#[test]
fn shutdown_is_idempotent() {
    let collector = Collector::start(200);

    let guard = with_pipeline(config_for(&collector), || {});
    guard.shutdown();
    // A shutdown hook may fire alongside `Drop`; neither may panic.
    guard.shutdown();
    guard.flush();
}

#[test]
fn an_unreachable_collector_does_not_break_the_server() {
    // Telemetry is not load-bearing: a collector that is down must not take
    // request handling with it.
    let dead = otel_support::dead_address();
    let guard = with_pipeline(
        OtelConfig::new("test-server")
            .with_endpoint(format!("http://{dead}"))
            .without_metrics()
            .with_shutdown_timeout(Duration::from_millis(500)),
        || {
            let span = rusty_log::info_span!("still-works");
            let _entered = span.enter();
            rusty_log::info!("the handler carries on");
        },
    );

    guard.shutdown();
}

#[test]
fn a_collector_that_rejects_the_batch_does_not_break_anything() {
    let collector = Collector::start(503);

    let guard = with_pipeline(config_for(&collector), || {
        drop(rusty_log::info_span!("refused"));
    });

    guard.shutdown();
    assert_eq!(collector.spans().len(), 1, "it was sent, and refused");
}

#[rusty_tokio::test(flavor = "multi_thread")]
async fn the_shutdown_hook_flushes() {
    let collector = Collector::start(200);

    let guard = Arc::new(with_pipeline(config_for(&collector), || {
        let span = rusty_log::info_span!("hooked");
        let _entered = span.enter();
        rusty_log::info!("inside");
    }));

    // Exactly what `ServerConfig::with_shutdown_hook` will call.
    let hook = guard.shutdown_hook();
    hook().await;

    assert!(
        collector.wait_until(Duration::from_secs(5), |c| !c.spans().is_empty()),
        "the shutdown hook should have flushed the buffer"
    );
}
