//! Exercises `RustyMcpServer` over a real MCP connection (an in-memory
//! duplex pair standing in for stdio/HTTP) rather than poking its methods
//! directly -- `RequestContext` isn't publicly constructible, and a real
//! client/server round trip is a better test of a `ServerHandler` impl
//! anyway: it also proves the handshake, tool schemas, and JSON-RPC framing
//! all actually work, not just the method bodies.

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use rp_mcp::{McpGateway, NativeTools, RustyMcpServer};
use rp_router::{Config, Router};

async fn connected_client() -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let router =
        Arc::new(Router::from_config(&Config::from_toml_str("providers = {}").unwrap()).await);
    let native = NativeTools::new(router);
    let gateway = Arc::new(McpGateway::empty());
    let handler = RustyMcpServer::new(native, gateway);

    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(service) = handler.serve(server_io).await {
            let _ = service.waiting().await;
        }
    });

    tokio::time::timeout(Duration::from_secs(5), ().serve(client_io))
        .await
        .expect("handshake timed out")
        .expect("client handshake failed")
}

#[tokio::test]
async fn lists_every_native_tool() {
    let client = connected_client().await;

    let tools = tokio::time::timeout(Duration::from_secs(5), client.peer().list_tools(None))
        .await
        .expect("list_tools timed out")
        .expect("list_tools failed");

    let mut names: Vec<String> = tools.tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, vec!["chat_completion", "embeddings", "list_models"]);

    let _ = client.cancel().await;
}

#[tokio::test]
async fn calling_list_models_returns_a_successful_result() {
    let client = connected_client().await;

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.call_tool(
            CallToolRequestParams::new("list_models")
                .with_arguments(serde_json::json!({}).as_object().cloned().unwrap()),
        ),
    )
    .await
    .expect("call_tool timed out")
    .expect("call_tool failed");

    assert_ne!(result.is_error, Some(true));

    let _ = client.cancel().await;
}

#[tokio::test]
async fn calling_an_unknown_upstream_prefixed_tool_is_a_protocol_error() {
    let client = connected_client().await;

    let error = tokio::time::timeout(
        Duration::from_secs(5),
        client.call_tool_once(CallToolRequestParams::new("no-such-upstream/some_tool")),
    )
    .await
    .expect("call_tool_once timed out")
    .expect_err("expected an error for an unknown upstream");

    assert!(error.to_string().contains("no-such-upstream"));

    let _ = client.cancel().await;
}
