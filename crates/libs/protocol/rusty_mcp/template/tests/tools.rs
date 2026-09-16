//! A real client talking to the server over an in-memory pipe.
//!
//! This is the same code path the stdio transport takes, so it exercises
//! dispatch, schema generation and serialization rather than just calling the
//! tool function directly.

use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientInfo, ProtocolVersion},
    service::RunningService,
};

#[path = "../src/server.rs"]
mod server;

use server::Server;

/// A client that asks for 2026-07-28.
///
/// `rmcp`'s default client still requests 2025-11-25, so a test that cares
/// about this revision's behaviour has to say so.
#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info
    }
}

async fn connect() -> RunningService<rmcp::RoleClient, Client> {
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    tokio::spawn(async move {
        let running = Server::new()
            .serve(server_transport)
            .await
            .expect("server starts");
        let _ = running.waiting().await;
    });

    Client
        .serve(client_transport)
        .await
        .expect("client connects")
}

fn call(name: &str, arguments: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_string()).with_arguments(
        arguments
            .as_object()
            .cloned()
            .expect("arguments are an object"),
    )
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .expect("the tool returns text")
}

#[tokio::test]
async fn the_tools_are_listed_with_their_schemas() {
    let client = connect().await;

    let tools = client.list_tools(None).await.expect("tools/list");
    let mut names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(names, ["divide", "greet"]);

    // 2026-07-28 requires cache hints on list results. Getting these means the
    // negotiated version really is the one `server_info` pins.
    assert!(tools.ttl_ms.is_some(), "missing ttlMs");
    assert!(tools.cache_scope.is_some(), "missing cacheScope");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn greet_uses_the_default_when_no_greeting_is_given() {
    let client = connect().await;

    let result = client
        .call_tool(call("greet", serde_json::json!({ "name": "world" })))
        .await
        .expect("call greet");
    assert_eq!(text_of(&result), "Hello, world!");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_bad_argument_is_a_protocol_error() {
    let client = connect().await;

    let err = client
        .call_tool(call("divide", serde_json::json!({ "a": 1, "b": 0 })))
        .await
        .expect_err("dividing by zero should fail");
    assert!(
        err.to_string().contains("divide by zero"),
        "unexpected error: {err}"
    );

    client.cancel().await.expect("cancel");
}
