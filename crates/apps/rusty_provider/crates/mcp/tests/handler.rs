#![allow(clippy::unwrap_used)]
//! The merged MCP server over real HTTP, driven by `rusty-mcp-client`: the
//! handshake, tool schemas, native tools, and the gateway proxying (and
//! reconnecting to) an upstream server.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rp_router::{Config, McpConfig, McpUpstreamConfig, McpUpstreamTransport, Router};
use rusty_mcp_client::proto::ContentBlock;
use rusty_mcp_client::{McpClient, McpClientError, McpServerSpec, McpTransport};
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{CallToolResult, ProtocolVersion, Tool};
use rusty_mcp_server::{bind_http, HttpConfig, Server};
use rusty_serve::{Limits, ShutdownHandle};

fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

fn text(s: &str) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(s)],
        ..CallToolResult::default()
    }
}

/// Serve `server` on `addr` from a thread of its own.
fn serve(server: Server, addr: &str) -> (SocketAddr, ShutdownHandle) {
    let http = bind_http(
        Arc::new(server),
        addr.parse().unwrap(),
        HttpConfig::default(),
        Limits::default(),
    )
    .unwrap();
    let bound = http.local_addr().unwrap();
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());
    (bound, stop)
}

fn upstream_server() -> Server {
    upstream_builder().build().unwrap()
}

/// Speaks only the classic revision, so the client holds a session: a
/// restarted server no longer knows it, and only a reconnect recovers.
fn classic_upstream() -> Server {
    upstream_builder()
        .versions(vec![ProtocolVersion::new(ProtocolVersion::V_2025_06_18)])
        .build()
        .unwrap()
}

fn upstream_builder() -> rusty_mcp_server::ServerBuilder {
    Server::builder("upstream", "1")
        .tool(Tool::new("echo", schema()), |_c, _p| Ok(text("echoed")))
        .tool(Tool::new("sleep", schema()), |_c, _p| {
            std::thread::sleep(Duration::from_secs(4));
            Ok(text("late"))
        })
}

fn spec(addr: SocketAddr) -> McpServerSpec {
    McpServerSpec {
        transport: McpTransport::Http,
        url: Some(format!("http://{addr}/mcp")),
        ..McpServerSpec::default()
    }
}

/// An rp-mcp server (no providers) proxying `upstreams`, served over HTTP;
/// a client connected to it.
async fn connected(
    upstreams: Vec<McpUpstreamConfig>,
    timeout_secs: u64,
) -> (McpClient, ShutdownHandle) {
    let router =
        Arc::new(Router::from_config(&Config::from_toml_str("providers = {}").unwrap()).await);
    let config = McpConfig {
        enabled: true,
        upstreams,
        reconnect_backoff_secs: 1,
        reconnect_backoff_max_secs: 1,
        timeout_secs,
        ..McpConfig::default()
    };
    let server = rp_mcp::build(&config, router).await.unwrap();
    let http = bind_http(
        server,
        "127.0.0.1:0".parse().unwrap(),
        HttpConfig::default(),
        Limits::default(),
    )
    .unwrap();
    let addr = http.local_addr().unwrap();
    let stop = http.shutdown_handle().unwrap();
    std::thread::spawn(move || http.run().unwrap());
    let client = McpClient::connect("rp", &spec(addr)).await.unwrap();
    (client, stop)
}

fn upstream_config(addr: SocketAddr) -> McpUpstreamConfig {
    McpUpstreamConfig {
        name: "up".to_owned(),
        transport: McpUpstreamTransport::Http {
            url: format!("http://{addr}/mcp"),
            bearer_token_env: None,
        },
    }
}

async fn names(client: &McpClient) -> Vec<String> {
    let mut names: Vec<String> = client
        .list_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    names.sort();
    names
}

/// Poll `names` until `done` holds or `limit` passes.
async fn wait_for(client: &McpClient, limit: Duration, done: impl Fn(&[String]) -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if done(&names(client).await) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lists_every_native_tool_with_its_schemas() {
    let (client, stop) = connected(vec![], 5).await;
    let tools = client.list_tools().await.unwrap();
    let mut listed: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    listed.sort_unstable();
    assert_eq!(listed, ["chat_completion", "embeddings", "list_models"]);
    for tool in &tools {
        assert!(tool.description.is_some(), "{}", tool.name);
        assert!(tool.output_schema.is_some(), "{}", tool.name);
        assert_eq!(tool.input_schema["type"].as_str(), Some("object"));
    }
    let chat = tools.iter().find(|t| t.name == "chat_completion").unwrap();
    assert!(chat.input_schema["required"]
        .as_array()
        .is_some_and(|r| r.len() == 2));
    stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_models_returns_structured_content() {
    let (client, stop) = connected(vec![], 5).await;
    let r = client
        .call_tool("list_models", serde_json::json!({}).as_object().cloned())
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true));
    assert!(r.structured_content.is_some_and(|v| v.as_array().is_some()));
    stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_native_arguments_are_an_error_not_a_crash() {
    let (client, stop) = connected(vec![], 5).await;
    let err = client
        .call_tool(
            "embeddings",
            serde_json::json!({ "model": 7 }).as_object().cloned(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, McpClientError::Service(m) if m.contains("invalid arguments")),
        "{err:?}"
    );
    stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calling_an_unknown_upstream_prefixed_tool_is_a_protocol_error() {
    let (client, stop) = connected(vec![], 5).await;
    let err = client
        .call_tool("no-such-upstream/some_tool", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no-such-upstream"), "{err}");
    stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upstreams_tools_are_listed_prefixed_and_callable() {
    let (addr, up_stop) = serve(upstream_server(), "127.0.0.1:0");
    let (client, stop) = connected(vec![upstream_config(addr)], 5).await;
    let listed = names(&client).await;
    assert!(listed.contains(&"up/echo".to_owned()), "{listed:?}");
    assert!(listed.contains(&"chat_completion".to_owned()));
    let r = client.call_tool("up/echo", None).await.unwrap();
    assert!(matches!(&r.content[0], ContentBlock::Text { text, .. } if text == "echoed"));
    stop.shutdown();
    up_stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hung_upstream_call_times_out_and_the_gateway_stays_usable() {
    let (addr, up_stop) = serve(upstream_server(), "127.0.0.1:0");
    let (client, stop) = connected(vec![upstream_config(addr)], 1).await;
    let t = Instant::now();
    let err = client.call_tool("up/sleep", None).await.unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    // Native tools never touch the upstream and answer at once.
    client.call_tool("list_models", None).await.unwrap();
    stop.shutdown();
    up_stop.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restarted_upstream_is_reconnected_by_the_supervisor() {
    let (addr, up_stop) = serve(classic_upstream(), "127.0.0.1:0");
    let (client, stop) = connected(vec![upstream_config(addr)], 2).await;
    assert!(names(&client).await.contains(&"up/echo".to_owned()));

    up_stop.shutdown();
    assert!(
        wait_for(&client, Duration::from_secs(15), |n| !n
            .contains(&"up/echo".to_owned()))
        .await,
        "the dead upstream stayed listed"
    );

    // Same address, new process state: the old session id is unknown to it.
    let (_, up_stop) = serve(classic_upstream(), &addr.to_string());
    assert!(
        wait_for(&client, Duration::from_secs(20), |n| n
            .contains(&"up/echo".to_owned()))
        .await,
        "the upstream never came back"
    );
    client.call_tool("up/echo", None).await.unwrap();
    stop.shutdown();
    up_stop.shutdown();
}
