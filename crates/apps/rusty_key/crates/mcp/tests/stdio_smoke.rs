//! End-to-end check of the `rmcp`-feature adapter against a real stdio MCP
//! server. Ignored by default (it needs a built server binary); run with
//!
//! ```text
//! cargo build -p rusty-mcp-demo
//! RK_MCP_SMOKE_SERVER=target/debug/rusty-mcp-demo \
//!     cargo test -p rk-mcp --features rmcp --test stdio_smoke -- --ignored
//! ```
#![cfg(feature = "rmcp")]

use rk_mcp::{client_from_spec, ServerSpec, Transport};

fn spec() -> ServerSpec {
    let command = std::env::var("RK_MCP_SMOKE_SERVER").expect("set RK_MCP_SMOKE_SERVER");
    ServerSpec {
        name: "demo".into(),
        transport: Transport::Stdio,
        command: Some(command),
        args: vec![],
        url: None,
        auth_token_env: None,
    }
}

#[tokio::test]
#[ignore = "needs RK_MCP_SMOKE_SERVER pointing at a built MCP server"]
async fn lists_tools_and_reconnects_over_stdio() {
    let client = client_from_spec(&spec()).await.expect("connect");
    let tools = client.list_tools().await.expect("list tools");
    assert!(!tools.is_empty(), "the server advertises at least one tool");
    assert!(tools.iter().all(|t| t.schema.is_object()));

    client.reconnect().await.expect("reconnect");
    let again = client.list_tools().await.expect("list after reconnect");
    assert_eq!(tools.len(), again.len());
}
