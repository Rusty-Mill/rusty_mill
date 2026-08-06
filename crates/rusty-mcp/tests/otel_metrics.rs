//! OTLP metric export.
//!
//! The unit tests cover which labels are chosen. What only shows up end to end
//! is whether they leave the process at all — and, more importantly, whether
//! the cardinality guard holds on the wire. A label the layer decided to drop
//! but the exporter sent anyway would be worth nothing.

#![cfg(feature = "otel")]

use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use axum::response::Response;
use http::Request;
use rusty_mcp::otel::{
    OtelConfig, OtelGuard,
    metrics::{McpMetricsLayer, Outcome, TaskOutcome},
};
use tower_layer::Layer as _;
use tower_service::Service as _;

/// Everything the fake collector was sent, concatenated.
type Received = Arc<Mutex<Vec<u8>>>;

/// A socket that records the bytes an OTLP exporter sends it.
///
/// Decoding protobuf properly would be its own project. Instrument names and
/// attribute values travel as length-prefixed UTF-8, so scanning the payload
/// for a distinctive string answers "did this reach the collector?" without a
/// decoder — and answers the negative case, which is what the cardinality tests
/// need, just as well.
async fn spawn_collector(received: Received) -> SocketAddr {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind collector");
    let addr = listener.local_addr().expect("addr");

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let received = Arc::clone(&received);

            tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match tokio::time::timeout(Duration::from_millis(400), socket.read(&mut buf))
                        .await
                    {
                        Ok(Ok(0)) | Err(_) => break,
                        Ok(Ok(n)) => received.lock().expect("lock").extend_from_slice(&buf[..n]),
                        Ok(Err(_)) => break,
                    }
                }
                // The gRPC client waits for a response; closing lets it finish
                // rather than hang until its own timeout.
                let _ = socket.shutdown().await;
            });
        }
    });

    addr
}

fn pipeline_to(addr: SocketAddr) -> OtelGuard {
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint(format!("http://{addr}"))
            // Long enough that nothing is pushed on a timer: every assertion
            // below is about what the explicit flush sends.
            .with_metrics_interval(Duration::from_secs(3600))
            .with_shutdown_timeout(Duration::from_secs(2)),
    )
    .expect("pipeline starts");

    guard
}

fn saw(received: &Received, needle: &str) -> bool {
    let bytes = received.lock().expect("lock");
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// A service that answers every request with `status`.
#[derive(Clone)]
struct Fixed(http::StatusCode);

impl<B> tower_service::Service<Request<B>> for Fixed {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request<B>) -> Self::Future {
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = self.0;
        std::future::ready(Ok(response))
    }
}

/// Drive one request through the layer.
async fn call_through(layer: &McpMetricsLayer, status: http::StatusCode, headers: &[(&str, &str)]) {
    let mut service = layer.layer(Fixed(status));

    let mut builder = Request::builder();
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(axum::body::Body::empty()).expect("request");

    let _ = service.call(request).await.expect("infallible");
}

#[tokio::test(flavor = "multi_thread")]
async fn instruments_reach_the_collector_after_a_flush() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let instruments = guard.instruments().expect("metrics are on by default");

    instruments.request_started("tools/call");
    instruments.request_finished("tools/call", Some("add"), Outcome::Ok, 0.012);

    // The flush is what sends them; without it the batch dies with the guard.
    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        saw(&received, "mcp.server.requests"),
        "the request counter never reached the collector"
    );
    assert!(
        saw(&received, "mcp.server.request.duration"),
        "the duration histogram never reached the collector"
    );
    assert!(saw(&received, "test-server"), "service.name is missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_layer_records_a_request_with_its_tool_name() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ))
    .with_known_names(["add"]);

    call_through(
        &layer,
        http::StatusCode::OK,
        &[("mcp-method", "tools/call"), ("mcp-name", "add")],
    )
    .await;

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(saw(&received, "mcp.server.requests"), "no counter arrived");
    assert!(saw(&received, "tools/call"), "the method label is missing");
    assert!(saw(&received, "add"), "the tool name label is missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tool_name_never_reaches_the_collector() {
    // The cardinality guarantee, proven on the wire rather than in a unit test.
    // Anyone who can reach the endpoint can call a tool that does not exist; if
    // that name became a label, they would own the label space.
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ))
    .with_known_names(["add"]);

    for i in 0..20 {
        call_through(
            &layer,
            http::StatusCode::OK,
            &[
                ("mcp-method", "tools/call"),
                ("mcp-name", &format!("forged-tool-{i:02}")),
            ],
        )
        .await;
    }

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        saw(&received, "mcp.server.requests"),
        "the requests should still have been counted"
    );
    assert!(
        !saw(&received, "forged-tool-"),
        "a client-supplied tool name was exported as a label"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_method_never_reaches_the_collector() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    call_through(
        &layer,
        http::StatusCode::OK,
        &[("mcp-method", "a-method-invented-by-a-client")],
    )
    .await;

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        !saw(&received, "a-method-invented-by-a-client"),
        "an unknown method was exported as a label"
    );
    assert!(saw(&received, "other"), "it should be bucketed as `other`");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_request_is_still_counted() {
    // The layer sits outside authorization precisely so this is true. A flood
    // of rejected tokens must not look like no traffic at all.
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    call_through(
        &layer,
        http::StatusCode::UNAUTHORIZED,
        &[("mcp-method", "tools/call")],
    )
    .await;

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        saw(&received, "unauthorized"),
        "a 401 should be counted with its own outcome"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn task_outcomes_are_counted() {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let instruments = guard.instruments().expect("metrics are enabled");

    instruments.task_started();
    instruments.task_finished(TaskOutcome::Completed);
    instruments.tasks_abandoned(3);

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(saw(&received, "mcp.server.tasks.started"));
    assert!(saw(&received, "mcp.server.tasks.finished"));
    assert!(saw(&received, "abandoned"), "the drain count is missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_can_be_turned_off() {
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint("http://127.0.0.1:1")
            .without_metrics(),
    )
    .expect("pipeline starts");

    assert!(guard.instruments().is_none());
    assert!(guard.meter_provider().is_none());

    // Neither shutdown nor flush may panic with no meter provider behind them.
    guard.flush();
    guard.shutdown();
}

/// A trivial MCP server, for the runtime-wiring tests below.
mod server {
    use rmcp::{
        ServerHandler,
        handler::server::{router::tool::ToolRouter, wrapper::Parameters},
        model::{ServerCapabilities, ServerInfo},
        tool, tool_handler, tool_router,
    };
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, JsonSchema)]
    pub struct EchoArgs {
        /// Text to echo back.
        pub message: String,
    }

    #[derive(Clone)]
    pub struct EchoServer {
        tool_router: ToolRouter<Self>,
    }

    #[tool_router(router = tool_router)]
    impl EchoServer {
        pub fn new() -> Self {
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
}

/// Start a server through `serve`, with metrics and optional authorization.
async fn spawn_server(
    layer: McpMetricsLayer,
    auth: Option<Arc<rusty_mcp::auth::AuthConfig>>,
) -> SocketAddr {
    use rusty_mcp::{HttpConfig, ServerConfig, Transport};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let config = ServerConfig {
        transport: Transport::Http(HttpConfig {
            bind: addr,
            sse_keep_alive: None,
            auth,
            metrics: Some(layer),
            ..Default::default()
        }),
        ..Default::default()
    };

    tokio::spawn(async move {
        let _ = rusty_mcp::serve(|| Ok(server::EchoServer::new()), config).await;
    });

    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return addr;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server never became ready");
}

/// POST a `tools/list`, with a bearer token if one is given.
async fn tools_list(addr: SocketAddr, token: Option<&str>) -> reqwest::StatusCode {
    let mut request = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/list")
        .body(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{
                 "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                 "io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        );

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    request
        .send()
        .await
        .expect("request reaches the server")
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_mounts_the_layer_on_the_endpoint() {
    // Proves the runtime wiring, not just the layer: `HttpConfig::metrics` has
    // to actually reach the mounted service.
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    let server = spawn_server(layer, None).await;
    assert!(tools_list(server, None).await.is_success());

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(saw(&received, "mcp.server.requests"), "no counter arrived");
    assert!(saw(&received, "tools/list"), "the method label is missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_rejected_by_the_auth_layer_is_still_counted() {
    // The ordering claim, end to end: metrics mount *outside* authorization, so
    // a request that never reaches the handler is still counted. Mounted the
    // other way round, a flood of bad tokens would look like no traffic at all.
    use rusty_mcp::auth::{AuthConfig, StaticTokenValidator, VerifiedToken};

    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_collector(Arc::clone(&received)).await;

    let guard = pipeline_to(addr);
    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    let validator = StaticTokenValidator::new().with_token(
        "good-token",
        VerifiedToken::new(["https://mcp.example.com/mcp"]),
    );
    let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(validator))
        .expect("valid resource")
        .with_authorization_servers(["https://auth.example.com"]);

    let server = spawn_server(layer, Some(Arc::new(auth))).await;

    let status = tools_list(server, None).await;
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);

    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        saw(&received, "unauthorized"),
        "the rejected request was never counted, so metrics sit inside the guard"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_collector_does_not_break_request_handling() {
    // Telemetry is not load-bearing.
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            // Port 1 is reliably closed.
            .with_endpoint("http://127.0.0.1:1")
            .with_shutdown_timeout(Duration::from_millis(200)),
    )
    .expect("the pipeline starts even if the collector is unreachable");

    let layer = McpMetricsLayer::new(Arc::clone(
        guard.instruments().expect("metrics are enabled"),
    ));

    call_through(
        &layer,
        http::StatusCode::OK,
        &[("mcp-method", "tools/list")],
    )
    .await;

    guard.shutdown();
}
