#![allow(clippy::unwrap_used)]
//! Trace context carried by MCP `_meta`.
//!
//! The unit tests cover parsing. These check the piece that only shows up on
//! the wire: that `_meta` actually carries the values across a real MCP
//! connection, and that a context survives being written back onto an
//! outbound request.

mod support;

use std::sync::{Arc, Mutex};

use rusty_json::{Map, json};
use rusty_mcp::trace::TraceContext;
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{HttpConfig, HttpHandler, Server};

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01";
const TRACESTATE: &str = "vendor=opaque";
const BAGGAGE: &str = "userId=alice,tier=gold";

type Observed = Arc<Mutex<Option<Option<TraceContext>>>>;

/// A server whose `observe` tool records the trace context of each call, read
/// from the request's `_meta` the way a gateway reads it.
async fn server(observed: Observed) -> std::net::SocketAddr {
    let mut schema = Value::object();
    schema.insert("type", "object");
    let server = Server::builder("tracing", "1")
        .tool(Tool::new("observe", schema), move |ctx, _call| {
            let extra = &ctx.meta().extra;
            let text = |k: &str| extra.get(k).and_then(Value::as_str);
            let seen = text("traceparent")
                .and_then(|tp| TraceContext::from_parts(tp, text("tracestate"), text("baggage")));
            *observed.lock().unwrap() = Some(seen.clone());
            Ok(CallToolResult {
                content: vec![ContentBlock::text(
                    seen.map_or_else(|| "none".to_owned(), |tc| tc.trace_id().to_owned()),
                )],
                ..CallToolResult::default()
            })
        })
        .build()
        .unwrap();
    let handler = Arc::new(HttpHandler::new(
        Arc::new(server),
        HttpConfig {
            path: support::PATH.to_owned(),
            ..HttpConfig::default()
        },
    ));
    support::serve(
        axum::Router::new().nest_service(support::PATH, rusty_mcp_axum::router(handler, 1 << 20)),
    )
    .await
}

/// Call `observe` with the given `_meta` trace values and return what the
/// server saw.
async fn round_trip(
    traceparent: Option<&str>,
    tracestate: Option<&str>,
    baggage: Option<&str>,
) -> Option<TraceContext> {
    let observed: Observed = Arc::new(Mutex::new(None));
    let addr = server(Arc::clone(&observed)).await;

    let mut meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    for (key, value) in [
        ("traceparent", traceparent),
        ("tracestate", tracestate),
        ("baggage", baggage),
    ] {
        if let Some(value) = value {
            meta[key] = json!(value);
        }
    }
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "observe", "arguments": {}, "_meta": meta }
    })
    .to_json_string();

    let response = support::post(addr, "tools/call", Some("observe"), body, None).await;
    assert!(response.status().is_success());

    let seen = observed.lock().unwrap().clone();
    seen.expect("the tool should have run")
}

#[tokio::test]
async fn the_server_sees_the_clients_trace_context() {
    let seen = round_trip(Some(TRACEPARENT), Some(TRACESTATE), Some(BAGGAGE))
        .await
        .expect("a trace context should have arrived");

    assert_eq!(seen.trace_id(), "0af7651916cd43dd8448eb211c80319c");
    assert_eq!(seen.parent_span_id(), "00f067aa0ba902b7");
    assert!(seen.is_sampled());
    assert_eq!(seen.tracestate(), Some(TRACESTATE));
    assert_eq!(seen.baggage().get("userId"), Some("alice"));
    assert_eq!(seen.baggage().get("tier"), Some("gold"));
}

#[tokio::test]
async fn a_request_without_trace_context_is_fine() {
    // Untraced clients must keep working; trace context is optional.
    assert!(round_trip(None, None, None).await.is_none());
}

#[tokio::test]
async fn a_malformed_traceparent_is_treated_as_absent() {
    // W3C says start a fresh trace rather than propagate something unparseable.
    // Critically, the call still succeeds — a broken upstream must not be able
    // to fail requests.
    assert!(
        round_trip(Some("garbage-not-a-traceparent"), None, None)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn tracestate_without_a_valid_traceparent_is_ignored() {
    // `tracestate` is only meaningful alongside a valid `traceparent`.
    assert!(
        round_trip(None, Some(TRACESTATE), Some(BAGGAGE))
            .await
            .is_none()
    );
}

#[test]
fn a_context_round_trips_back_onto_an_outbound_request() {
    let meta: Map = json!({
        "traceparent": TRACEPARENT, "tracestate": TRACESTATE, "baggage": BAGGAGE
    })
    .as_object()
    .unwrap()
    .clone();
    let seen = TraceContext::from_meta(&meta).expect("context");

    // Simulate propagating onward: new span id, same trace.
    let child = seen.child("1111111111111111").expect("valid span id");
    let mut outbound = Map::new();
    child.apply_to(&mut outbound);

    assert_eq!(
        outbound["traceparent"],
        "00-0af7651916cd43dd8448eb211c80319c-1111111111111111-01"
    );
    assert_eq!(outbound["tracestate"], TRACESTATE);

    // Baggage survives the trip, whatever order it was written in.
    let reparsed = TraceContext::from_meta(&outbound).expect("reparses");
    assert_eq!(reparsed.baggage().get("userId"), Some("alice"));
    assert_eq!(reparsed.trace_id(), seen.trace_id());
}

#[test]
fn non_string_trace_fields_are_ignored() {
    let meta = json!({ "traceparent": 7 }).as_object().unwrap().clone();
    assert!(TraceContext::from_meta(&meta).is_none());
}
