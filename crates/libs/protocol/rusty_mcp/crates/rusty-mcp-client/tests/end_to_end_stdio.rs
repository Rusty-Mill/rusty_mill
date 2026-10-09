#![allow(clippy::unwrap_used)]
//! `McpClient` over stdio against a real child process: the native client's
//! example server.

use rusty_mcp_client::client::{McpClient, McpServerSpec, McpTransport};
use rusty_mcp_client::proto::ContentBlock;
use std::path::PathBuf;

/// The example server, built once (`cargo test` alone does not build examples).
fn server_binary() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().unwrap().parent().unwrap();
    let mut build = std::process::Command::new(env!("CARGO"));
    build.args([
        "build",
        "--example",
        "server",
        "-p",
        "rusty_mcp_client_native",
    ]);
    if profile_dir.file_name().is_some_and(|n| n == "release") {
        build.arg("--release");
    }
    assert!(
        build.status().unwrap().success(),
        "building the example failed"
    );
    profile_dir
        .join("examples")
        .join(format!("server{}", std::env::consts::EXE_SUFFIX))
}

#[tokio::test]
async fn stdio_connects_calls_and_shuts_down() {
    let spec = McpServerSpec {
        transport: McpTransport::Stdio,
        command: server_binary().to_string_lossy().into_owned(),
        ..McpServerSpec::default()
    };
    let client = McpClient::connect("child", &spec).await.unwrap();
    assert_eq!(client.list_tools().await.unwrap()[0].name, "add");
    let args = serde_json::json!({ "a": 40, "b": 2 }).as_object().cloned();
    let r = client.call_tool("add", args).await.unwrap();
    assert!(matches!(&r.content[0], ContentBlock::Text { text, .. } if text == "42"));
    tokio::time::timeout(std::time::Duration::from_secs(8), client.shutdown())
        .await
        .expect("shutdown hung")
        .unwrap();
}
