//! Load shedding over a real socket.
//!
//! The unit tests drive the layer directly. These answer the questions only the
//! whole stack can: does `serve` actually mount it, does shedding happen ahead
//! of the authorization check, and — the one worth being empirical about —
//! does a timeout kill a long-lived `subscriptions/listen`?
//!
//! That last one cannot be settled by reading the layer. It depends on whether
//! the transport returns the SSE response promptly and streams afterwards, or
//! holds the response future open for the life of the subscription. If it were
//! the latter, a timeout would silently break change notifications for anyone
//! who enabled one.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use rusty_mcp::{
    HttpConfig, ServerConfig, Transport, limits::LimitsLayer, subscriptions::ChangeBroadcaster,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Arguments for the `sleep` tool.
#[derive(Debug, Deserialize, JsonSchema)]
struct SleepArgs {
    /// How long to block for, in milliseconds.
    ms: u64,
}

/// A server with one deliberately slow tool.
#[derive(Clone)]
struct SlowServer {
    changes: ChangeBroadcaster,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl SlowServer {
    fn new(changes: ChangeBroadcaster) -> Self {
        Self {
            changes,
            tool_router: Self::tool_router(),
        }
    }

    /// Runs inline — no tasks extension — so the request is held open for the
    /// whole duration. That is what makes it useful here.
    #[tool(description = "Block for a while.")]
    async fn sleep(&self, Parameters(SleepArgs { ms }): Parameters<SleepArgs>) -> String {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        "done".to_string()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SlowServer {
    fn get_info(&self) -> ServerInfo {
        rusty_mcp::server_info(
            "slow-server",
            "0.1.0",
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .enable_resources_list_changed()
                .build(),
        )
    }

    rusty_mcp::forward_subscription_methods!(changes);
}

async fn spawn_server(
    limits: LimitsLayer,
    auth: Option<Arc<rusty_mcp::auth::AuthConfig>>,
) -> (SocketAddr, ChangeBroadcaster) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let changes = ChangeBroadcaster::new();

    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            auth,
            limits: Some(limits),
            ..Default::default()
        }),
        ..Default::default()
    };

    let handler_changes = changes.clone();
    tokio::spawn(async move {
        let _ =
            rusty_mcp::serve(move || Ok(SlowServer::new(handler_changes.clone())), config).await;
    });

    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return (addr, changes);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server never became ready");
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
}

/// POST a JSON-RPC body and return the response.
///
/// `Mcp-Name` is not optional for `tools/call` — the transport rejects the
/// request with `-32020` without it (SEP-2243).
async fn post(
    addr: SocketAddr,
    method: &str,
    body: String,
    token: Option<&str>,
) -> reqwest::Response {
    let mut request = client()
        .post(format!("http://{addr}/mcp"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method)
        .body(body);

    if method == "tools/call" {
        request = request.header("Mcp-Name", "sleep");
    }
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    request.send().await.expect("request reaches the server")
}

fn call_sleep(ms: u64) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{
             "name":"sleep","arguments":{{"ms":{ms}}},"_meta":{{
             "io.modelcontextprotocol/protocolVersion":"2026-07-28",
             "io.modelcontextprotocol/clientCapabilities":{{}}}}}}}}"#
    )
}

fn tools_list() -> String {
    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{
         "io.modelcontextprotocol/protocolVersion":"2026-07-28",
         "io.modelcontextprotocol/clientCapabilities":{}}}}"#
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_mounts_the_limits_layer() {
    let (addr, _changes) = spawn_server(LimitsLayer::new().with_max_concurrent(1), None).await;

    let held = tokio::spawn(async move { post(addr, "tools/call", call_sleep(400), None).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let shed = post(addr, "tools/list", tools_list(), None).await;
    assert_eq!(shed.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        shed.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("1"),
        "a shed response should tell the client when to come back"
    );

    let first = held.await.expect("join");
    assert!(first.status().is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn capacity_returns_once_the_slow_request_finishes() {
    // A limit that did not release would turn one slow call into an outage.
    let (addr, _changes) = spawn_server(LimitsLayer::new().with_max_concurrent(1), None).await;

    assert!(
        post(addr, "tools/call", call_sleep(50), None)
            .await
            .status()
            .is_success()
    );

    for _ in 0..3 {
        assert!(
            post(addr, "tools/list", tools_list(), None)
                .await
                .status()
                .is_success(),
            "capacity should have come back"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shedding_happens_before_the_token_is_checked() {
    // The ordering claim. Limits sit outside authorization, so an unauthorized
    // request that arrives while the server is full is shed with `503` and
    // never costs a validation — under a flood, that difference is the point.
    use rusty_mcp::auth::{AuthConfig, StaticTokenValidator, VerifiedToken};

    let validator = StaticTokenValidator::new().with_token(
        "good-token",
        VerifiedToken::new(["https://mcp.example.com/mcp"]),
    );
    let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(validator))
        .expect("valid resource")
        .with_authorization_servers(["https://auth.example.com"]);

    let (addr, _changes) = spawn_server(
        LimitsLayer::new().with_max_concurrent(1),
        Some(Arc::new(auth)),
    )
    .await;

    // Fill the one slot with an authorized request.
    let held =
        tokio::spawn(
            async move { post(addr, "tools/call", call_sleep(400), Some("good-token")).await },
        );
    tokio::time::sleep(Duration::from_millis(100)).await;

    // No token at all. If auth ran first this would be a 401.
    let response = post(addr, "tools/list", tools_list(), None).await;
    assert_eq!(
        response.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "a 401 here would mean the token was validated before capacity was checked"
    );

    let first = held.await.expect("join");
    assert!(first.status().is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inline_tool_that_runs_too_long_is_cut_off() {
    let (addr, _changes) = spawn_server(
        LimitsLayer::new().with_timeout(Duration::from_millis(150)),
        None,
    )
    .await;

    let response = post(addr, "tools/call", call_sleep(5_000), None).await;
    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_inside_the_timeout_is_untouched() {
    let (addr, _changes) = spawn_server(
        LimitsLayer::new().with_timeout(Duration::from_secs(10)),
        None,
    )
    .await;

    let response = post(addr, "tools/call", call_sleep(20), None).await;
    assert!(response.status().is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_lived_subscription_survives_a_short_timeout() {
    // The empirical question, and the reason this file exists rather than
    // trusting the layer's unit tests. `subscriptions/listen` is long-lived by
    // design; if the timeout wrapped the whole subscription rather than the
    // response that starts it, enabling a timeout would silently break change
    // notifications for everyone who turned one on.
    //
    // Driven with the real client rather than a hand-rolled SSE request, so
    // this exercises the same path a real subscriber takes.
    use rmcp::{
        ClientLifecycleMode, ClientServiceExt,
        model::{ClientInfo, ProtocolVersion, SubscriptionFilter},
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };

    const TIMEOUT: Duration = Duration::from_millis(200);

    let (addr, changes) = spawn_server(LimitsLayer::new().with_timeout(TIMEOUT), None).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")),
    );
    let client = ClientInfo::default()
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("client connects");

    let mut subscription = client
        .listen(
            SubscriptionFilter::builder()
                .resources_list_changed()
                .build(),
        )
        .await
        .expect("the subscription should be accepted");

    // Well past the timeout, so a timeout covering the whole subscription
    // would already have killed it.
    tokio::time::sleep(TIMEOUT * 5).await;

    changes.resources_changed();

    let notification = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("an event should arrive long after the timeout would have fired")
        .expect("the subscription should still be live")
        .expect("not the end of the stream");

    let _ = notification;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_limits_configured_leaves_the_server_unbounded() {
    // The default. Nothing here should change for a server that never opts in.
    let (addr, _changes) = spawn_server(LimitsLayer::new(), None).await;

    let mut handles = Vec::new();
    for _ in 0..8 {
        handles.push(tokio::spawn(async move {
            post(addr, "tools/call", call_sleep(100), None).await
        }));
    }

    for handle in handles {
        assert!(handle.await.expect("join").status().is_success());
    }
}
