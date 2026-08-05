//! End-to-end tests: a real client talking to [`DemoServer`] over an in-memory
//! duplex pipe, exercising the same code path as the stdio transport.

use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientInfo, ProtocolVersion},
    service::RunningService,
};

// The binary's modules, compiled into the test.
#[path = "../src/prompts.rs"]
mod prompts;
#[path = "../src/resources.rs"]
mod resources;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/tools/mod.rs"]
mod tools;

use server::DemoServer;

/// A client that asks for a specific protocol revision.
///
/// `rmcp`'s default client still requests 2025-11-25, so tests that care about
/// 2026-07-28 behaviour have to say so explicitly.
#[derive(Clone)]
struct VersionedClient(ProtocolVersion);

impl ClientHandler for VersionedClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = self.0.clone();
        info
    }
}

/// Connect a 2026-07-28 client to a freshly served [`DemoServer`].
async fn connect() -> RunningService<rmcp::RoleClient, VersionedClient> {
    connect_as(ProtocolVersion::V_2026_07_28).await
}

/// Connect a client pinned to `version`.
async fn connect_as(version: ProtocolVersion) -> RunningService<rmcp::RoleClient, VersionedClient> {
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    tokio::spawn(async move {
        let running = DemoServer::new()
            .serve(server_transport)
            .await
            .expect("server should start");
        let _ = running.waiting().await;
    });

    VersionedClient(version)
        .serve(client_transport)
        .await
        .expect("client should connect")
}

/// Build a tool call. `CallToolRequestParams` is `#[non_exhaustive]`, so it is
/// constructed through its builder rather than a struct literal.
fn call(name: &'static str, args: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name).with_arguments(
        args.as_object()
            .cloned()
            .expect("tool arguments must be a JSON object"),
    )
}

/// Pull the JSON payload out of a tool result's structured content.
fn structured(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    result
        .structured_content
        .clone()
        .expect("tool should return structured content")
}

#[tokio::test]
async fn lists_tools_from_every_router() {
    let client = connect().await;

    let tools = client.list_tools(None).await.expect("tools/list");
    let mut names: Vec<_> = tools.tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();

    // Every router contributes, merged by the `+` chain in
    // `DemoServer::with_state_and_tasks`.
    assert_eq!(
        names,
        ["add", "countdown", "divide", "slugify", "text_stats"]
    );

    // Every tool carries a description and an input schema; without these the
    // model has nothing to select on.
    for tool in &tools.tools {
        assert!(
            tool.description.as_ref().is_some_and(|d| !d.is_empty()),
            "tool {} is missing a description",
            tool.name
        );
    }

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn list_tools_carries_cache_hints() {
    let client = connect().await;

    let tools = client.list_tools(None).await.expect("tools/list");

    // The scaffold pins the advertised revision, so negotiation lands on 2026-07-28.
    assert_eq!(
        client.peer_info().map(|i| i.protocol_version.clone()),
        Some(ProtocolVersion::V_2026_07_28),
    );

    // SEP-2549: `tools/list` results must carry cache hints under 2026-07-28.
    assert!(
        tools.ttl_ms.is_some(),
        "expected a ttlMs cache hint on tools/list"
    );
    assert!(
        tools.cache_scope.is_some(),
        "expected a cacheScope cache hint on tools/list"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn add_returns_structured_output() {
    let client = connect().await;

    let result = client
        .call_tool(call("add", serde_json::json!({ "a": 2, "b": 40 })))
        .await
        .expect("call add");

    assert_ne!(result.is_error, Some(true));
    assert_eq!(structured(&result)["sum"], 42);

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn state_is_shared_across_calls() {
    let client = connect().await;

    let mut seen = Vec::new();
    for _ in 0..3 {
        let result = client
            .call_tool(call("add", serde_json::json!({ "a": 1, "b": 1 })))
            .await
            .expect("call add");
        seen.push(structured(&result)["calls"].as_u64().expect("calls is u64"));
    }

    // The Arc<DemoState> outlives individual calls.
    assert_eq!(seen, vec![1, 2, 3]);

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn divide_reports_quotient_and_remainder() {
    let client = connect().await;

    let result = client
        .call_tool(call("divide", serde_json::json!({ "a": 17, "b": 5 })))
        .await
        .expect("call divide");

    let value = structured(&result);
    assert_eq!(value["quotient"], 3);
    assert_eq!(value["remainder"], 2);

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn divide_by_zero_is_a_protocol_error() {
    let client = connect().await;

    let err = client
        .call_tool(call("divide", serde_json::json!({ "a": 1, "b": 0 })))
        .await
        .expect_err("divide by zero should fail");

    assert!(
        err.to_string().contains("divide by zero"),
        "unexpected error: {err}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn add_overflow_is_rejected_rather_than_panicking() {
    let client = connect().await;

    let err = client
        .call_tool(call("add", serde_json::json!({ "a": i64::MAX, "b": 1 })))
        .await
        .expect_err("overflow should fail");

    assert!(
        err.to_string().contains("overflow"),
        "unexpected error: {err}"
    );

    // The server survived the bad call and still answers.
    let tools = client
        .list_tools(None)
        .await
        .expect("tools/list after error");
    assert_eq!(tools.tools.len(), 5);

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn text_tools_work() {
    let client = connect().await;

    let slug = client
        .call_tool(call(
            "slugify",
            serde_json::json!({ "text": "Hello, MCP World!" }),
        ))
        .await
        .expect("call slugify");

    let text = slug
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("slugify returns text");
    assert_eq!(text, "hello-mcp-world");

    let stats = client
        .call_tool(call(
            "text_stats",
            serde_json::json!({ "text": "one two\nthree" }),
        ))
        .await
        .expect("call text_stats");

    let value = structured(&stats);
    assert_eq!(value["words"], 3);
    assert_eq!(value["lines"], 2);

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn older_clients_still_negotiate_down() {
    // Pinning 2026-07-28 must not lock out clients on the previous revision.
    let client = connect_as(ProtocolVersion::V_2025_11_25).await;

    assert_eq!(
        client.peer_info().map(|i| i.protocol_version.clone()),
        Some(ProtocolVersion::V_2025_11_25),
    );

    let result = client
        .call_tool(call("add", serde_json::json!({ "a": 20, "b": 22 })))
        .await
        .expect("call add on 2025-11-25");
    assert_eq!(structured(&result)["sum"], 42);

    client.cancel().await.expect("cancel");
}
