//! End-to-end tests for `mcpGuardrails`.
//!
//! A real MCP client, over a real socket, through the gateway, into a real
//! subprocess MCP server — with a scripted gRPC policy processor consulted in
//! between. What the unit tests in `agentgateway-mcp` cannot reach is exactly
//! what matters here: that a processor is actually consulted at the right
//! point, that a rewrite it returns is what the client ends up seeing, and
//! that a refusal stops the call before the upstream is touched.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use agentgateway_mcp::{
    AuthorizationError, McpRequest, McpRequestResult, McpResponse, McpResponseResult,
    authorization_error, request_result, response_result,
};
use prost::Message as _;
use rmcp::{
    ServiceExt, model::CallToolRequestParams, service::RunningService,
    transport::StreamableHttpClientTransport,
};
use tokio_util::sync::CancellationToken;

/// How the scripted processor should answer.
#[derive(Clone)]
enum Script {
    Pass,
    /// Answer with this JSON in place of the body it was given.
    Rewrite(String),
    /// Refuse.
    Refuse(&'static str),
    /// Pass, asking for `x-user-id` on the upstream request.
    SetHeaders,
}

/// What the processor was asked.
#[derive(Default)]
struct Seen {
    requests: Mutex<Vec<(String, Option<String>)>>,
    responses: Mutex<Vec<(String, String)>>,
    calls: AtomicUsize,
}

async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind");
    listener.local_addr().expect("should have an addr").port()
}

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("examples");
    path.push("mock_mcp_server");
    assert!(path.exists(), "fixture not built at {}", path.display());
    path.display().to_string()
}

/// A scripted `ExtMcp` gRPC server.
async fn processor(script: Script) -> (String, Arc<Seen>, CancellationToken) {
    use axum::body::Body;
    use http_body_util::BodyExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("processor should bind");
    let addr: SocketAddr = listener.local_addr().expect("should have an address");
    let seen = Arc::new(Seen::default());
    let shutdown = CancellationToken::new();

    let recorder = Arc::clone(&seen);
    let stopping = shutdown.clone();

    let app = axum::Router::new().fallback(axum::routing::any(
        move |request: axum::extract::Request| {
            let script = script.clone();
            let recorder = Arc::clone(&recorder);
            async move {
                let is_request = request.uri().path().ends_with("CheckRequest");
                let body = request
                    .into_body()
                    .collect()
                    .await
                    .map(|b| b.to_bytes())
                    .unwrap_or_default();
                // gRPC frames a message as a compression flag and a 4-byte
                // big-endian length, then the protobuf.
                let message = &body[5..];

                recorder.calls.fetch_add(1, Ordering::Relaxed);

                let payload = if is_request {
                    let decoded = McpRequest::decode(message).expect("should decode");
                    if let Ok(mut seen) = recorder.requests.lock() {
                        seen.push((
                            decoded.method,
                            decoded
                                .mcp_request
                                .map(|b| String::from_utf8_lossy(&b).into_owned()),
                        ));
                    }
                    McpRequestResult {
                        result: Some(match &script {
                            Script::Rewrite(body) => {
                                request_result::Result::Mutated(body.as_bytes().to_vec())
                            }
                            Script::Refuse(reason) => {
                                request_result::Result::Error(AuthorizationError {
                                    code: authorization_error::Code::PermissionDenied as i32,
                                    reason: (*reason).to_string(),
                                    mcp_error: None,
                                })
                            }
                            Script::Pass | Script::SetHeaders => {
                                request_result::Result::Pass(Default::default())
                            }
                        }),
                        header_mutation: matches!(script, Script::SetHeaders).then(|| {
                            agentgateway_mcp::HeaderMutation {
                                set: vec![agentgateway_mcp::McpHeader {
                                    key: "x-user-id".into(),
                                    value: b"u-42".to_vec(),
                                }],
                                remove: Vec::new(),
                            }
                        }),
                        ..Default::default()
                    }
                    .encode_to_vec()
                } else {
                    let decoded = McpResponse::decode(message).expect("should decode");
                    if let Ok(mut seen) = recorder.responses.lock() {
                        seen.push((
                            decoded.method,
                            String::from_utf8_lossy(&decoded.mcp_response).into_owned(),
                        ));
                    }
                    McpResponseResult {
                        result: Some(match &script {
                            Script::Rewrite(body) => {
                                response_result::Result::Mutated(body.as_bytes().to_vec())
                            }
                            Script::Refuse(reason) => {
                                response_result::Result::Error(AuthorizationError {
                                    code: authorization_error::Code::PermissionDenied as i32,
                                    reason: (*reason).to_string(),
                                    mcp_error: None,
                                })
                            }
                            Script::Pass | Script::SetHeaders => {
                                response_result::Result::Pass(Default::default())
                            }
                        }),
                    }
                    .encode_to_vec()
                };

                let mut framed = Vec::with_capacity(payload.len() + 5);
                framed.push(0);
                framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                framed.extend_from_slice(&payload);

                http::Response::builder()
                    .status(200)
                    .header("content-type", "application/grpc")
                    .header("grpc-status", "0")
                    .body(Body::from(framed))
                    .expect("response should build")
            }
        },
    ));

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await;
    });

    (addr.to_string(), seen, shutdown)
}

struct Harness {
    client: RunningService<rmcp::RoleClient, ()>,
    shutdown: CancellationToken,
}

impl Harness {
    /// Boot a gateway whose route carries `methods` on one processor at `host`.
    async fn start(host: &str, methods: &str) -> Harness {
        let port = free_port().await;
        let yaml = format!(
            r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: mcp
            matches:
              - path:
                  pathPrefix: /mcp
            policies:
              mcpGuardrails:
                processors:
                  - kind: remote
                    host: "{host}"
                    timeout: 5s
                    methods: {methods}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_TOOLS: "echo,ping"
"#,
            server = mock_server()
        );

        let config = Config::from_yaml(&yaml).expect("config should parse");
        config.validate().expect("config should validate");
        let gateway = Gateway::build(&config, None)
            .await
            .expect("gateway should build");

        let shutdown = CancellationToken::new();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
        let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
            .await
            .expect("gateway should bind");

        let client = ()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "http://127.0.0.1:{port}/mcp"
            )))
            .await
            .expect("client should complete the MCP handshake");

        Harness { client, shutdown }
    }

    async fn tool_names(&self) -> Result<Vec<String>, String> {
        let mut names: Vec<String> = self
            .client
            .list_all_tools()
            .await
            .map_err(|err| err.to_string())?
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        Ok(names)
    }

    async fn call(&self, name: &str) -> Result<String, String> {
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(name.to_string()))
            .await
            .map_err(|err| err.to_string())?;
        Ok(result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join(""))
    }

    async fn stop(self) {
        let _ = self.client.cancel().await;
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn a_processor_sees_the_call_and_lets_it_through() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::start(&host, r#"{ "tools/call": full }"#).await;

    assert_eq!(harness.call("alpha_echo").await, Ok("alpha:echo".into()));

    let requests = seen.requests.lock().expect("lock").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "tools/call");
    let params = requests[0].1.as_deref().expect("params should be sent");
    assert!(
        params.contains(r#""name":"echo""#),
        "a processor should see the unmuxed name the upstream will get, not \
         the federated one; got {params}"
    );

    assert_eq!(seen.responses.lock().expect("lock").len(), 1);
    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_refusal_stops_the_call_before_the_upstream() {
    let (host, seen, stop) = processor(Script::Refuse("blocked by policy")).await;
    let harness = Harness::start(&host, r#"{ "tools/call": request }"#).await;

    let err = harness
        .call("alpha_echo")
        .await
        .expect_err("a refused call should not succeed");
    assert!(err.contains("blocked by policy"), "got: {err}");
    assert_eq!(
        seen.responses.lock().expect("lock").len(),
        0,
        "the response phase must not run for a call that never happened"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_rewrite_the_request() {
    // The point of a guardrail rather than a gate: redirecting `echo` to
    // `ping` is not something a yes/no answer can express.
    let (host, _, stop) = processor(Script::Rewrite(r#"{"name":"ping"}"#.into())).await;
    let harness = Harness::start(&host, r#"{ "tools/call": request }"#).await;

    assert_eq!(
        harness.call("alpha_echo").await,
        Ok("alpha:ping".into()),
        "the upstream should have received the rewritten params"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_rewrite_the_result() {
    let (host, _, stop) = processor(Script::Rewrite(
        r#"{"content":[{"type":"text","text":"[redacted]"}],"isError":false}"#.into(),
    ))
    .await;
    let harness = Harness::start(&host, r#"{ "tools/call": response }"#).await;

    assert_eq!(
        harness.call("alpha_echo").await,
        Ok("[redacted]".into()),
        "the client should see what the processor returned, not the upstream's result"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_filter_the_catalogue_on_the_response_side() {
    let (host, seen, stop) = processor(Script::Rewrite(
        r#"{"tools":[{"name":"alpha_echo","description":"","inputSchema":{"type":"object"}}]}"#
            .into(),
    ))
    .await;
    let harness = Harness::start(&host, r#"{ "tools/list": response }"#).await;

    assert_eq!(
        harness.tool_names().await,
        Ok(vec!["alpha_echo".to_string()]),
        "alpha_ping should have been filtered out by the processor"
    );

    let responses = seen.responses.lock().expect("lock").clone();
    assert_eq!(responses[0].0, "tools/list");
    assert!(
        responses[0].1.contains("alpha_ping"),
        "the processor should have been shown the full merged catalogue: {}",
        responses[0].1
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_method_the_processor_is_not_keyed_on_is_not_sent_to_it() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::start(&host, r#"{ "tools/call": full }"#).await;

    harness.tool_names().await.expect("tools/list should work");
    assert_eq!(
        seen.calls.load(Ordering::Relaxed),
        0,
        "tools/list is not on this processor's method list"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn an_unreachable_processor_refuses_the_call() {
    // The whole policy turns on this: a policy service that is down must not
    // become an open door.
    let dead = free_port().await;
    let harness =
        Harness::start(&format!("127.0.0.1:{dead}"), r#"{ "tools/call": request }"#).await;

    let err = harness
        .call("alpha_echo")
        .await
        .expect_err("an unreachable processor must not let the call through");
    assert!(err.contains("mcpGuardrails"), "got: {err}");

    harness.stop().await;
}

#[tokio::test]
async fn failing_open_has_to_be_asked_for() {
    let dead = free_port().await;
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            policies:
              mcpGuardrails:
                processors:
                  - host: "127.0.0.1:{dead}"
                    failureMode: failOpen
                    timeout: 2s
                    methods: {{ "tools/call": request }}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
"#,
        server = mock_server()
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");
    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .expect("client should complete the MCP handshake");

    let result = client
        .call_tool(CallToolRequestParams::new("alpha_echo".to_string()))
        .await
        .expect("failOpen serves what the processor never approved -- deliberately");
    assert!(
        result
            .content
            .iter()
            .any(|block| block.as_text().is_some_and(|t| t.text == "alpha:echo"))
    );

    let _ = client.cancel().await;
    shutdown.cancel();
}

#[tokio::test]
async fn a_processor_that_cannot_be_addressed_fails_at_startup() {
    // Rather than turning every call into a confusing refusal at runtime.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              mcpGuardrails:
                processors:
                  - host: "not a host"
                    methods: {{ "tools/call": full }}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
"#,
        server = mock_server()
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    let err = Gateway::build(&config, None)
        .await
        .err()
        .expect("an unaddressable processor should be a startup failure");
    assert!(err.to_string().contains("processors[0]"), "got: {err}");
}

/// The headers each request to an HTTP target carried, in arrival order.
type SeenHeaders = Arc<Mutex<Vec<Vec<(String, String)>>>>;

/// A Streamable HTTP MCP server that records the headers each request carried.
///
/// The stdio fixture cannot serve this test: `headerMutation` changes the
/// upstream *HTTP* request, and a subprocess speaking over a pipe has none.
async fn http_target() -> (
    u16,
    Arc<Mutex<Vec<Vec<(String, String)>>>>,
    CancellationToken,
) {
    use rmcp::{
        ErrorData as McpError, ServerHandler,
        model::{
            CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
            PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
        },
        service::{RequestContext, RoleServer},
        transport::streamable_http_server::{
            StreamableHttpService, session::local::LocalSessionManager,
        },
    };

    #[derive(Clone)]
    struct Echo;

    impl ServerHandler for Echo {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
        }

        async fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, McpError> {
            let mut schema = serde_json::Map::new();
            schema.insert("type".into(), serde_json::Value::String("object".into()));
            Ok(ListToolsResult {
                tools: vec![Tool::new("echo", "echo", Arc::new(schema))],
                ..Default::default()
            })
        }

        async fn call_tool(
            &self,
            _request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, McpError> {
            Ok(CallToolResponse::Complete(CallToolResult::success(vec![
                ContentBlock::text("served"),
            ])))
        }
    }

    let port = free_port().await;
    let seen: SeenHeaders = Arc::new(Mutex::new(Vec::new()));
    let shutdown = CancellationToken::new();

    let service = StreamableHttpService::new(
        || Ok(Echo),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let recorder = Arc::clone(&seen);
    let app = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let recorder = Arc::clone(&recorder);
                async move {
                    if let Ok(mut seen) = recorder.lock() {
                        seen.push(
                            request
                                .headers()
                                .iter()
                                .map(|(k, v)| {
                                    (
                                        k.as_str().to_string(),
                                        v.to_str().unwrap_or_default().to_string(),
                                    )
                                })
                                .collect(),
                        );
                    }
                    next.run(request).await
                }
            },
        ));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("target should bind");
    let stopping = shutdown.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await;
    });

    (port, seen, shutdown)
}

/// Boot a gateway with one HTTP MCP target behind a guardrail.
async fn start_with_http_target(
    host: &str,
    methods: &str,
    target_port: u16,
) -> (u16, CancellationToken) {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            policies:
              mcpGuardrails:
                processors:
                  - host: "{host}"
                    timeout: 5s
                    methods: {methods}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      mcp:
                        host: 127.0.0.1
                        port: {target_port}
                        path: /mcp
"#
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (port, shutdown)
}

#[tokio::test]
async fn a_header_mutation_reaches_the_upstream_request() {
    // The load-bearing assumption of the whole feature: the change rides in
    // the request's extensions, which rmcp carries in memory from the peer
    // down to the transport. Nothing but a real call proves that.
    let (target_port, target_headers, stop_target) = http_target().await;
    let (host, _, stop_proc) = processor(Script::SetHeaders).await;
    let (port, shutdown) =
        start_with_http_target(&host, r#"{ "tools/call": request }"#, target_port).await;

    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://127.0.0.1:{port}/mcp"
        )))
        .await
        .expect("client should complete the MCP handshake");

    let result = client
        .call_tool(CallToolRequestParams::new("alpha_echo".to_string()))
        .await
        .expect("the call should succeed");
    assert!(
        result
            .content
            .iter()
            .any(|b| b.as_text().is_some_and(|t| t.text == "served"))
    );

    let seen = target_headers.lock().expect("lock").clone();
    let last = seen.last().expect("the target should have been called");
    let get = |name: &str| last.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    assert_eq!(
        get("x-user-id"),
        Some("u-42".to_string()),
        "the processor's header should be on the upstream request; saw {last:?}"
    );

    // The handshake happened before any processor was consulted, so it must
    // not carry the header -- this is a per-call change, not a connection one.
    let handshake = seen.first().expect("there should be a handshake request");
    assert!(
        !handshake.iter().any(|(k, _)| k == "x-user-id"),
        "the header should not have leaked onto the connection: {handshake:?}"
    );

    let _ = client.cancel().await;
    shutdown.cancel();
    stop_proc.cancel();
    stop_target.cancel();
}

#[tokio::test]
async fn a_header_mutation_on_a_stdio_target_is_dropped_not_fatal() {
    // A pipe has no headers. The call should still succeed rather than
    // failing because a processor asked for something that cannot apply.
    let (host, _, stop) = processor(Script::SetHeaders).await;
    let harness = Harness::start(&host, r#"{ "tools/call": request }"#).await;

    assert_eq!(harness.call("alpha_echo").await, Ok("alpha:echo".into()));

    stop.cancel();
    harness.stop().await;
}
