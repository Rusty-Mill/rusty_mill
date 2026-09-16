//! Streamable HTTP transport tests.
//!
//! These go over a real TCP socket with the `rmcp` client, so they exercise the
//! whole path: `serve` → axum → `StreamableHttpService` → handler.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use rmcp::{
    ClientLifecycleMode, ClientServiceExt, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolRequestParams, ClientInfo, ProtocolVersion, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use rusty_mcp::{HttpConfig, ServerConfig, Transport};
use schemars::JsonSchema;
use serde::Deserialize;

/// Arguments for the `echo` tool.
#[derive(Debug, Deserialize, JsonSchema)]
struct EchoArgs {
    /// Text to echo back.
    message: String,
}

/// Minimal server with a single tool.
#[derive(Clone)]
struct EchoServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl EchoServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Echo the message back.")]
    async fn echo(&self, Parameters(EchoArgs { message }): Parameters<EchoArgs>) -> String {
        message
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info(
            "echo-server",
            "0.1.0",
            ServerCapabilities::builder().enable_tools().build(),
        )
    }
}

/// Claim an ephemeral port, then release it for the server to bind.
async fn free_port() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    listener.local_addr().expect("local addr")
}

/// Start the demo server on a free port and return its MCP endpoint URL.
async fn spawn_server() -> String {
    let addr = free_port().await;
    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            ..Default::default()
        }),
        ..Default::default()
    };

    tokio::spawn(async move {
        let _ = rusty_mcp::serve(|| Ok(EchoServer::new()), config).await;
    });

    let url = format!("http://{addr}/mcp");
    wait_until_ready(addr).await;
    url
}

/// Poll the listener until it accepts, so tests do not race the bind.
async fn wait_until_ready(addr: SocketAddr) {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server at {addr} never became ready");
}

/// Connect a stateless 2026-07-28 client.
///
/// `ClientLifecycleMode::Discover` is what makes the client probe with
/// `server/discover` and then carry `_meta` (protocol version, capabilities) on
/// every request — the stateless startup SEP-2575 replaced `initialize` with.
/// The default `serve` still uses the legacy handshake.
async fn connect(url: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ClientInfo> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url.to_string()),
    );

    ClientInfo::default()
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("client should connect over http")
}

#[tokio::test]
async fn serves_tools_over_streamable_http() {
    let url = spawn_server().await;
    let client = connect(&url).await;

    let tools = client.list_tools(None).await.expect("tools/list over http");
    assert_eq!(tools.tools.len(), 1);
    assert_eq!(tools.tools[0].name, "echo");

    let result = client
        .call_tool(
            CallToolRequestParams::new("echo").with_arguments(
                serde_json::json!({ "message": "over the wire" })
                    .as_object()
                    .cloned()
                    .expect("object"),
            ),
        )
        .await
        .expect("call echo over http");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("echo returns text");
    assert_eq!(text, "over the wire");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn independent_connections_need_no_shared_session() {
    // SEP-2567 removed protocol sessions. Two clients hitting the same server
    // must each work standalone — this is what lets the server sit behind a
    // round-robin load balancer with no affinity.
    let url = spawn_server().await;

    let first = connect(&url).await;
    let second = connect(&url).await;

    for client in [&first, &second] {
        let tools = client.list_tools(None).await.expect("tools/list");
        assert_eq!(tools.tools.len(), 1);

        // Cache hints are required on list results under 2026-07-28.
        assert!(tools.ttl_ms.is_some(), "missing ttlMs");
        assert!(tools.cache_scope.is_some(), "missing cacheScope");
    }

    first.cancel().await.expect("cancel first");
    second.cancel().await.expect("cancel second");
}

#[tokio::test]
async fn rejects_requests_with_a_disallowed_host() {
    // The default allow-list is loopback-only, guarding against DNS rebinding.
    let addr = free_port().await;
    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            ..Default::default()
        }),
        ..Default::default()
    };
    tokio::spawn(async move {
        let _ = rusty_mcp::serve(|| Ok(EchoServer::new()), config).await;
    });
    wait_until_ready(addr).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/mcp"))
        .header("Host", "evil.example.com")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
        .send()
        .await
        .expect("request should reach the server");

    assert!(
        response.status().is_client_error(),
        "expected a 4xx for a disallowed Host, got {}",
        response.status()
    );
}

#[tokio::test]
async fn shared_state_survives_across_requests() {
    // Streamable HTTP builds a handler per request, so any state that must
    // outlive a call belongs in an Arc captured by the factory.
    #[derive(Clone)]
    struct CountingServer {
        hits: Arc<std::sync::atomic::AtomicU64>,
        tool_router: ToolRouter<Self>,
    }

    #[tool_router(router = tool_router)]
    impl CountingServer {
        #[tool(description = "Return how many times this tool has been called.")]
        async fn hit(&self) -> String {
            let n = self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            n.to_string()
        }
    }

    #[tool_handler(router = self.tool_router)]
    impl ServerHandler for CountingServer {
        fn get_info(&self) -> ServerInfo {
            rusty_mcp::server_info(
                "counting-server",
                "0.1.0",
                ServerCapabilities::builder().enable_tools().build(),
            )
        }
    }

    let addr = free_port().await;
    let hits = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            ..Default::default()
        }),
        ..Default::default()
    };

    tokio::spawn({
        let hits = Arc::clone(&hits);
        async move {
            let _ = rusty_mcp::serve(
                move || {
                    Ok(CountingServer {
                        hits: Arc::clone(&hits),
                        tool_router: CountingServer::tool_router(),
                    })
                },
                config,
            )
            .await;
        }
    });
    wait_until_ready(addr).await;

    let url = format!("http://{addr}/mcp");
    let mut seen = Vec::new();
    for _ in 0..3 {
        // A fresh connection each time: no session, no affinity.
        let client = connect(&url).await;
        let result = client
            .call_tool(CallToolRequestParams::new("hit"))
            .await
            .expect("call hit");
        seen.push(
            result
                .content
                .first()
                .and_then(|c| c.as_text())
                .map(|t| t.text.clone())
                .expect("hit returns text"),
        );
        client.cancel().await.expect("cancel");
    }

    assert_eq!(seen, vec!["1", "2", "3"]);
}
