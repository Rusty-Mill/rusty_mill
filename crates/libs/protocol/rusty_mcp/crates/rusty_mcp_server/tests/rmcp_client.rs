//! Acceptance: an independent client (`rmcp`, a dev-dependency used only as an
//! oracle) drives the `rusty-mcp-echo` fixture over a real child process.

use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use tokio::process::Command;

fn args(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

fn text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test(flavor = "current_thread")]
async fn rmcp_client_initializes_lists_and_calls() {
    let command = Command::new(env!("CARGO_BIN_EXE_rusty-mcp-echo"));
    let transport = TokioChildProcess::new(command).expect("spawns the fixture");
    let client = ().serve(transport).await.expect("initialize handshake");

    let info = client.peer_info().expect("server info after initialize");
    let server = info.server_info.as_ref().expect("serverInfo is present");
    assert_eq!(server.name, "rusty-mcp-echo");

    let tools = client.list_all_tools().await.expect("tools/list");
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, ["echo", "add", "fail", "slow"]);
    assert!(tools[0].input_schema.get("properties").is_some());

    let echoed = client
        .call_tool(
            CallToolRequestParams::new("echo").with_arguments(args(&[("text", "hello".into())])),
        )
        .await
        .expect("echo");
    assert_eq!(text(&echoed), "hello");

    let sum = client
        .call_tool(
            CallToolRequestParams::new("add")
                .with_arguments(args(&[("a", 2.into()), ("b", 40.into())])),
        )
        .await
        .expect("add");
    assert_eq!(text(&sum), "42");

    let failed = client
        .call_tool(CallToolRequestParams::new("fail"))
        .await
        .expect("a failing tool is still a successful RPC");
    assert_eq!(failed.is_error, Some(true));
    assert_eq!(text(&failed), "this tool always fails");

    let unknown = client.call_tool(CallToolRequestParams::new("nope")).await;
    assert!(unknown.is_err(), "unknown tool is a protocol error");

    client.cancel().await.expect("clean shutdown");
}
