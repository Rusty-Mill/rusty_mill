//! Acceptance: an independent client (`rmcp`, a dev-dependency used only as an
//! oracle) drives the `rusty-mcp-echo` fixture over a real child process.

use rmcp::model::{
    ArgumentInfo, CallToolRequestParams, CompleteRequestParams, GetPromptRequestParams,
    ReadResourceRequestParams, Reference, ResourceContents,
};
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

#[tokio::test(flavor = "current_thread")]
async fn rmcp_client_reads_resources_prompts_and_completions() {
    let command = Command::new(env!("CARGO_BIN_EXE_rusty-mcp-echo"));
    let transport = TokioChildProcess::new(command).expect("spawns the fixture");
    let client = ().serve(transport).await.expect("initialize handshake");

    let caps = client
        .peer_info()
        .expect("server info")
        .capabilities
        .clone();
    assert!(caps.resources.is_some() && caps.prompts.is_some() && caps.completions.is_some());

    let resources = client.list_all_resources().await.expect("resources/list");
    assert_eq!(resources[0].uri, "mem://greeting");
    let templates = client
        .list_all_resource_templates()
        .await
        .expect("templates");
    assert_eq!(templates[0].uri_template, "mem://notes/{id}");

    let read = |uri: &str| ReadResourceRequestParams::new(uri);
    let fixed = client
        .read_resource(read("mem://greeting"))
        .await
        .expect("read");
    assert!(
        matches!(&fixed.contents[0], ResourceContents::TextResourceContents { text, .. } if text == "hello")
    );
    let templated = client
        .read_resource(read("mem://notes/7"))
        .await
        .expect("read");
    assert!(
        matches!(&templated.contents[0], ResourceContents::TextResourceContents { text, .. } if text == "note 7")
    );
    assert!(client.read_resource(read("mem://nope")).await.is_err());

    let prompts = client.list_all_prompts().await.expect("prompts/list");
    assert_eq!(prompts[0].name, "greet");
    let got = client
        .get_prompt(
            GetPromptRequestParams::new("greet").with_arguments(args(&[("name", "Ada".into())])),
        )
        .await
        .expect("prompts/get");
    assert_eq!(got.messages.len(), 1);
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("greet"))
            .await
            .is_err(),
        "required argument missing"
    );

    let completed = client
        .complete(CompleteRequestParams::new(
            Reference::for_prompt("greet"),
            ArgumentInfo::new("name", "al"),
        ))
        .await
        .expect("completion/complete");
    assert_eq!(completed.completion.values, ["alice", "alex"]);
    assert_eq!(completed.completion.total, Some(2));

    client.cancel().await.expect("clean shutdown");
}
