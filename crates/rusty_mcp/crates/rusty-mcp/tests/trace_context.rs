//! Trace context travelling over a real MCP connection.
//!
//! The unit tests cover parsing; these check the piece that only shows up on
//! the wire — that `_meta` actually carries the values across, and that the
//! server sees what the client sent.

use std::sync::{Arc, Mutex};

use rmcp::{
    ClientHandler, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolRequestParams, ClientInfo, ProtocolVersion, RequestParamsMeta, ServerCapabilities,
        ServerInfo,
    },
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
};
use rusty_mcp::trace::TraceContext;
use schemars::JsonSchema;
use serde::Deserialize;

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01";
const TRACESTATE: &str = "vendor=opaque";
const BAGGAGE: &str = "userId=alice,tier=gold";

/// Empty arguments.
#[derive(Debug, Deserialize, JsonSchema)]
struct NoArgs {}

/// What the server observed, so the test can assert on it.
type Observed = Arc<Mutex<Option<Option<TraceContext>>>>;

/// A server that records the trace context of each call.
#[derive(Clone)]
struct TracingServer {
    observed: Observed,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl TracingServer {
    #[tool(description = "Record the caller's trace context.")]
    async fn observe(
        &self,
        Parameters(_): Parameters<NoArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> String {
        let seen = TraceContext::from_request(&ctx);
        *self.observed.lock().expect("lock") = Some(seen.clone());

        seen.map(|tc| tc.trace_id().to_string())
            .unwrap_or_else(|| "none".to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TracingServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info(
            "tracing-server",
            "0.1.0",
            ServerCapabilities::builder().enable_tools().build(),
        )
    }
}

#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info
    }
}

/// Call `observe` with the given `_meta` trace values and return what the
/// server saw.
async fn round_trip(
    traceparent: Option<&str>,
    tracestate: Option<&str>,
    baggage: Option<&str>,
) -> Option<TraceContext> {
    let observed: Observed = Arc::new(Mutex::new(None));

    let (server_transport, client_transport) = tokio::io::duplex(4096);
    tokio::spawn({
        let observed = Arc::clone(&observed);
        async move {
            let running = TracingServer {
                observed,
                tool_router: TracingServer::tool_router(),
            }
            .serve(server_transport)
            .await
            .expect("server starts");
            let _ = running.waiting().await;
        }
    });

    let client = Client
        .serve(client_transport)
        .await
        .expect("client connects");

    let mut params = CallToolRequestParams::new("observe");
    if let Some(traceparent) = traceparent {
        params.set_traceparent(traceparent);
    }
    if let Some(tracestate) = tracestate {
        params.set_tracestate(tracestate);
    }
    if let Some(baggage) = baggage {
        params.set_baggage(baggage);
    }

    client.call_tool(params).await.expect("call observe");
    client.cancel().await.expect("cancel");

    let seen = observed.lock().expect("lock").clone();
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

#[tokio::test]
async fn a_context_round_trips_back_onto_an_outbound_request() {
    let seen = round_trip(Some(TRACEPARENT), Some(TRACESTATE), Some(BAGGAGE))
        .await
        .expect("context");

    // Simulate propagating onward: new span id, same trace.
    let child = seen.child("1111111111111111").expect("valid span id");
    let mut outbound = CallToolRequestParams::new("downstream");
    child.apply_to(&mut outbound);

    assert_eq!(
        outbound.traceparent(),
        Some("00-0af7651916cd43dd8448eb211c80319c-1111111111111111-01")
    );
    assert_eq!(outbound.tracestate(), Some(TRACESTATE));

    // Baggage survives the trip, whatever order it was written in.
    let reparsed = TraceContext::from_meta(&outbound).expect("reparses");
    assert_eq!(reparsed.baggage().get("userId"), Some("alice"));
    assert_eq!(reparsed.trace_id(), seen.trace_id());
}
