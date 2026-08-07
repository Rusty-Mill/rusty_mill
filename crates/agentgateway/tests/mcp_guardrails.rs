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
    ServiceExt,
    model::{CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams},
    service::RunningService,
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
    /// Pass, attaching a metadata bag.
    Annotate,
}

/// What the processor was asked.
#[derive(Default)]
struct Seen {
    requests: Mutex<Vec<(String, Option<String>)>>,
    responses: Mutex<Vec<(String, String)>>,
    /// `metadata_context` per call, flattened to string values.
    metadata: Mutex<Vec<Vec<(String, String)>>>,
    calls: AtomicUsize,
}

/// A `metadata_context` struct as plain key/value strings.
fn flatten(context: Option<prost_types::Struct>) -> Vec<(String, String)> {
    context
        .into_iter()
        .flat_map(|s| s.fields)
        .filter_map(|(k, v)| match v.kind {
            Some(prost_types::value::Kind::StringValue(text)) => Some((k, text)),
            _ => None,
        })
        .collect()
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
                    if let Ok(mut seen) = recorder.metadata.lock() {
                        seen.push(flatten(decoded.metadata_context));
                    }
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
                            Script::Pass | Script::SetHeaders | Script::Annotate => {
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
                        metadata: matches!(script, Script::Annotate)
                            .then(|| {
                                match agentgateway_mcp::to_proto_value(serde_json::json!({
                                    "classification": "phishing",
                                }))
                                .kind
                                {
                                    Some(prost_types::value::Kind::StructValue(s)) => Some(s),
                                    _ => None,
                                }
                            })
                            .flatten(),
                    }
                    .encode_to_vec()
                } else {
                    let decoded = McpResponse::decode(message).expect("should decode");
                    if let Ok(mut seen) = recorder.metadata.lock() {
                        seen.push(flatten(decoded.metadata_context));
                    }
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
                            Script::Pass | Script::SetHeaders | Script::Annotate => {
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
                          MOCK_PROMPTS: "summarize,leak"
                          MOCK_RESOURCES: "memo:insights,file:///secret"
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

// ---------------------------------------------------------------------------
// Prompts and resources
//
// The same processor chain, over the other four methods this gateway serves.
// What is worth testing separately is not that the plumbing was duplicated --
// it is shared -- but the two places where prompts and resources behave
// differently from tools: what a processor is shown, and what a rewrite of a
// listing has to look like.
// ---------------------------------------------------------------------------

impl Harness {
    async fn prompt_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .client
            .list_all_prompts()
            .await
            .expect("prompts/list should work")
            .into_iter()
            .map(|p| p.name)
            .collect();
        names.sort();
        names
    }

    async fn get_prompt(&self, name: &str) -> Result<String, String> {
        let result = self
            .client
            .get_prompt(GetPromptRequestParams::new(name))
            .await
            .map_err(|err| err.to_string())?;
        Ok(result
            .messages
            .iter()
            .filter_map(|m| m.content.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join(""))
    }

    async fn resource_uris(&self) -> Vec<String> {
        let mut uris: Vec<String> = self
            .client
            .list_all_resources()
            .await
            .expect("resources/list should work")
            .into_iter()
            .map(|r| r.uri)
            .collect();
        uris.sort();
        uris
    }

    async fn read_resource(&self, uri: &str) -> Result<String, String> {
        let result = self
            .client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|err| err.to_string())?;
        match result.contents.first().expect("one content block") {
            rmcp::model::ResourceContents::TextResourceContents { text, .. } => Ok(text.clone()),
            other => panic!("expected text contents, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_processor_sees_a_prompt_fetch_by_its_unmuxed_name() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::start(&host, r#"{ "prompts/get": full }"#).await;

    assert_eq!(
        harness.get_prompt("alpha_summarize").await,
        Ok("alpha:summarize".into())
    );

    let requests = seen.requests.lock().expect("lock").clone();
    assert_eq!(requests[0].0, "prompts/get");
    let params = requests[0].1.as_deref().expect("params should be sent");
    assert!(
        params.contains(r#""name":"summarize""#),
        "a processor should see the name the upstream will get, not the \
         federated one; got {params}"
    );

    assert_eq!(seen.responses.lock().expect("lock").len(), 1);
    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_sees_a_resource_read_by_its_unmuxed_uri() {
    // The same split as names, and the one most likely to be got wrong: a
    // processor matching on `alpha+memo:insights` would never fire.
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::start(&host, r#"{ "resources/read": request }"#).await;

    assert_eq!(
        harness.read_resource("alpha+memo:insights").await,
        Ok("alpha:memo:insights".into())
    );

    let requests = seen.requests.lock().expect("lock").clone();
    assert_eq!(requests[0].0, "resources/read");
    let params = requests[0].1.as_deref().expect("params should be sent");
    assert!(
        params.contains(r#""uri":"memo:insights""#),
        "a processor should see the target's own URI; got {params}"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_refuse_a_prompt_before_the_upstream() {
    let (host, seen, stop) = processor(Script::Refuse("prompts are off")).await;
    let harness = Harness::start(&host, r#"{ "prompts/get": request }"#).await;

    let err = harness
        .get_prompt("alpha_summarize")
        .await
        .expect_err("a refused fetch should not succeed");
    assert!(err.contains("prompts are off"), "got: {err}");
    assert_eq!(
        seen.responses.lock().expect("lock").len(),
        0,
        "the response phase must not run for a fetch that never happened"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_refuse_a_resource_read() {
    let (host, _, stop) = processor(Script::Refuse("that one is sealed")).await;
    let harness = Harness::start(&host, r#"{ "resources/read": request }"#).await;

    let err = harness
        .read_resource("alpha+memo:insights")
        .await
        .expect_err("a refused read should not succeed");
    assert!(err.contains("that one is sealed"), "got: {err}");

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_redact_a_resource_it_let_through() {
    // The case that makes this a guardrail rather than a gate: the read is
    // allowed, and what comes back is not what the upstream sent.
    let (host, _, stop) = processor(Script::Rewrite(
        r#"{"contents":[{"uri":"alpha+memo:insights","mimeType":"text/plain","text":"[redacted]"}]}"#
            .into(),
    ))
    .await;
    let harness = Harness::start(&host, r#"{ "resources/read": response }"#).await;

    assert_eq!(
        harness.read_resource("alpha+memo:insights").await,
        Ok("[redacted]".into())
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_rewrite_a_prompts_messages() {
    let (host, _, stop) = processor(Script::Rewrite(
        r#"{"messages":[{"role":"user","content":{"type":"text","text":"replaced"}}]}"#.into(),
    ))
    .await;
    let harness = Harness::start(&host, r#"{ "prompts/get": response }"#).await;

    assert_eq!(
        harness.get_prompt("alpha_summarize").await,
        Ok("replaced".into())
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_filter_the_prompt_listing() {
    let (host, seen, stop) = processor(Script::Rewrite(
        r#"{"prompts":[{"name":"alpha_summarize","description":"kept"}]}"#.into(),
    ))
    .await;
    let harness = Harness::start(&host, r#"{ "prompts/list": response }"#).await;

    assert_eq!(harness.prompt_names().await, vec!["alpha_summarize"]);

    let responses = seen.responses.lock().expect("lock").clone();
    assert!(
        responses[0].1.contains("alpha_leak"),
        "the processor should have been shown the full merged listing: {}",
        responses[0].1
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_filter_the_resource_listing() {
    let (host, seen, stop) = processor(Script::Rewrite(
        r#"{"resources":[{"uri":"alpha+memo:insights","name":"memo:insights"}]}"#.into(),
    ))
    .await;
    let harness = Harness::start(&host, r#"{ "resources/list": response }"#).await;

    assert_eq!(harness.resource_uris().await, vec!["alpha+memo:insights"]);

    // The listing a processor is shown carries the *federated* URIs -- the
    // response phase sees what the client would get, which is the opposite of
    // the request phase and worth knowing when writing a filter.
    let responses = seen.responses.lock().expect("lock").clone();
    assert!(
        responses[0].1.contains("alpha+file:///secret"),
        "got: {}",
        responses[0].1
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_wildcard_can_hook_every_prompt_and_resource_method() {
    // The point of the `prefix/*` form: one processor over a whole namespace.
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::start(&host, r#"{ "resources/*": request }"#).await;

    harness.resource_uris().await;
    harness.read_resource("alpha+memo:insights").await.ok();
    let _ = harness
        .client
        .list_all_resource_templates()
        .await
        .expect("resources/templates/list should work");

    let mut methods: Vec<String> = seen
        .requests
        .lock()
        .expect("lock")
        .iter()
        .map(|(m, _)| m.clone())
        .collect();
    methods.sort();
    methods.dedup();
    assert_eq!(
        methods,
        vec![
            "resources/list",
            "resources/read",
            "resources/templates/list"
        ],
        "`resources/*` should reach all three"
    );

    // And nothing outside the namespace.
    harness.prompt_names().await;
    assert!(
        !seen
            .requests
            .lock()
            .expect("lock")
            .iter()
            .any(|(m, _)| m.starts_with("prompts/")),
        "a prefix wildcard must not reach another namespace"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn an_unreachable_processor_refuses_a_prompt_fetch_too() {
    // Failing closed is not a tools-only property.
    let dead = free_port().await;
    let harness = Harness::start(
        &format!("127.0.0.1:{dead}"),
        r#"{ "prompts/get": request, "resources/read": request }"#,
    )
    .await;

    let err = harness
        .get_prompt("alpha_summarize")
        .await
        .expect_err("an unreachable processor must not let the fetch through");
    assert!(err.contains("mcpGuardrails"), "got: {err}");

    let err = harness
        .read_resource("alpha+memo:insights")
        .await
        .expect_err("an unreachable processor must not let the read through");
    assert!(err.contains("mcpGuardrails"), "got: {err}");

    harness.stop().await;
}

#[tokio::test]
async fn a_list_rewrite_on_the_request_phase_is_discarded_for_prompts_too() {
    // `prompts/list` carries no params, so there is nothing to rewrite on the
    // way in. The listing should come back whole rather than mangled.
    let (host, _, stop) = processor(Script::Rewrite(r#"{"nonsense":true}"#.into())).await;
    let harness = Harness::start(&host, r#"{ "prompts/list": request }"#).await;

    assert_eq!(
        harness.prompt_names().await,
        vec!["alpha_leak", "alpha_summarize"]
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_header_mutation_applies_to_a_resource_read() {
    // Header changes are a request-phase property of the chain, not of
    // tools/call -- but a stdio target has no headers, so this only checks the
    // call still succeeds. The HTTP-target case is covered for tools.
    let (host, _, stop) = processor(Script::SetHeaders).await;
    let harness = Harness::start(&host, r#"{ "resources/read": request }"#).await;

    assert_eq!(
        harness.read_resource("alpha+memo:insights").await,
        Ok("alpha:memo:insights".into())
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_rewrite_which_prompt_is_fetched() {
    // The request phase can redirect, not only refuse. Upstream is explicit
    // that a rewrite is not re-authorized, so this is a deliberate hole in the
    // gate and worth having pinned rather than discovered.
    let (host, _, stop) = processor(Script::Rewrite(r#"{"name":"leak"}"#.into())).await;
    let harness = Harness::start(&host, r#"{ "prompts/get": request }"#).await;

    assert_eq!(
        harness.get_prompt("alpha_summarize").await,
        Ok("alpha:leak".into()),
        "the upstream should have been asked for the rewritten name"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_rewrite_which_resource_is_read() {
    let (host, _, stop) = processor(Script::Rewrite(r#"{"uri":"file:///secret"}"#.into())).await;
    let harness = Harness::start(&host, r#"{ "resources/read": request }"#).await;

    assert_eq!(
        harness.read_resource("alpha+memo:insights").await,
        Ok("alpha:file:///secret".into())
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_rewritten_uri_that_carries_the_targets_own_prefix_is_unwrapped() {
    // A processor handed `memo:insights` may well hand back the federated form
    // it saw on the listing. The upstream only knows its own URIs, so the
    // prefix is stripped rather than passed through to fail.
    let (host, _, stop) =
        processor(Script::Rewrite(r#"{"uri":"alpha+file:///secret"}"#.into())).await;
    let harness = Harness::start(&host, r#"{ "resources/read": request }"#).await;

    assert_eq!(
        harness.read_resource("alpha+memo:insights").await,
        Ok("alpha:file:///secret".into())
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_processor_can_refuse_a_listing_outright() {
    for (methods, refused) in [
        (r#"{ "prompts/list": request }"#, "prompts"),
        (r#"{ "resources/list": request }"#, "resources"),
        (r#"{ "resources/templates/list": request }"#, "templates"),
    ] {
        let (host, _, stop) = processor(Script::Refuse("no listing for you")).await;
        let harness = Harness::start(&host, methods).await;

        let err = match refused {
            "prompts" => harness.client.list_prompts(None).await.err(),
            "resources" => harness.client.list_resources(None).await.err(),
            _ => harness.client.list_resource_templates(None).await.err(),
        };
        let err = err
            .expect("a refused listing should not succeed")
            .to_string();
        assert!(err.contains("no listing for you"), "{refused}: {err}");

        stop.cancel();
        harness.stop().await;
    }
}

#[tokio::test]
async fn rules_are_applied_before_a_processor_is_consulted() {
    // A processor should only be asked about calls that were otherwise going
    // to happen -- the same ordering tools use. Otherwise a guardrail would be
    // billed for traffic the route had already refused.
    let (host, seen, stop) = processor(Script::Pass).await;
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
              mcpAuthorization:
                rules:
                  - 'mcp.prompt.name == "summarize"'
              mcpGuardrails:
                processors:
                  - host: "{host}"
                    timeout: 5s
                    methods: {{ "prompts/get": full }}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_PROMPTS: "summarize,leak"
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

    let err = client
        .get_prompt(GetPromptRequestParams::new("alpha_leak"))
        .await
        .expect_err("the rule should have refused this")
        .to_string();
    assert!(err.contains("not permitted"), "got: {err}");
    assert_eq!(
        seen.calls.load(Ordering::Relaxed),
        0,
        "a processor must not be consulted about a call the rules already refused"
    );

    let _ = client.cancel().await;
    shutdown.cancel();
    stop.cancel();
}

#[tokio::test]
async fn the_response_phase_sees_a_resource_read_re_qualified() {
    // The asymmetry: the request phase shows a processor the upstream's own
    // URI, the response phase shows what the client will get. A filter written
    // against one form will not match the other.
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::start(&host, r#"{ "resources/read": response }"#).await;

    harness.read_resource("alpha+memo:insights").await.ok();

    let responses = seen.responses.lock().expect("lock").clone();
    assert!(
        responses[0].1.contains("alpha+memo:insights"),
        "contents reach the response phase already re-qualified: {}",
        responses[0].1
    );

    stop.cancel();
    harness.stop().await;
}

// ---------------------------------------------------------------------------
// `metadata` naming the subject
// ---------------------------------------------------------------------------

impl Harness {
    /// Boot a gateway with one processor carrying `metadata` expressions.
    async fn with_metadata(host: &str, methods: &str, metadata: &str) -> Harness {
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
                    metadata:
{metadata}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_TOOLS: "echo"
                          MOCK_PROMPTS: "summarize"
                          MOCK_RESOURCES: "memo:insights"
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
}

fn sent(seen: &Seen, index: usize) -> Vec<(String, String)> {
    let mut fields = seen.metadata.lock().expect("lock")[index].clone();
    fields.sort();
    fields
}

#[tokio::test]
async fn metadata_names_the_prompt_end_to_end() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::with_metadata(
        &host,
        r#"{ "prompts/get": request }"#,
        "                      prompt: 'mcp.prompt.name'\n                      target: 'mcp.prompt.target'",
    )
    .await;

    assert_eq!(
        harness.get_prompt("alpha_summarize").await,
        Ok("alpha:summarize".into())
    );

    assert_eq!(
        sent(&seen, 0),
        vec![
            ("prompt".to_string(), "summarize".to_string()),
            ("target".to_string(), "alpha".to_string()),
        ],
        "the processor should be told which prompt, unmuxed, and whose"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn metadata_names_the_resource_end_to_end() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::with_metadata(
        &host,
        r#"{ "resources/read": request }"#,
        "                      uri: 'mcp.resource.name'",
    )
    .await;

    harness.read_resource("alpha+memo:insights").await.ok();

    assert_eq!(
        sent(&seen, 0),
        vec![("uri".to_string(), "memo:insights".to_string())],
        "the target's own URI, matching what the request body carries"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn metadata_names_the_tool_end_to_end() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::with_metadata(
        &host,
        r#"{ "tools/call": request }"#,
        "                      tool: 'mcp.tool.name'",
    )
    .await;

    harness.call("alpha_echo").await.ok();

    assert_eq!(
        sent(&seen, 0),
        vec![("tool".to_string(), "echo".to_string())]
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn a_listing_sends_no_subject_but_keeps_the_rest() {
    let (host, seen, stop) = processor(Script::Pass).await;
    let harness = Harness::with_metadata(
        &host,
        r#"{ "prompts/list": request }"#,
        "                      prompt: 'mcp.prompt.name'\n                      method: 'request.method'",
    )
    .await;

    harness.prompt_names().await;

    assert_eq!(
        sent(&seen, 0),
        vec![("method".to_string(), "prompts/list".to_string())],
        "a fanout has no single subject, so that key is dropped rather than invented"
    );

    stop.cancel();
    harness.stop().await;
}

#[tokio::test]
async fn the_response_phase_names_what_was_actually_fetched() {
    // A request-phase rewrite changes what the result is about, and the
    // response phase should describe the result it is looking at rather than
    // the name the client happened to ask for.
    let (host, seen, stop) = processor(Script::Rewrite(r#"{"name":"summarize"}"#.into())).await;
    let harness = Harness::with_metadata(
        &host,
        r#"{ "prompts/get": full }"#,
        "                      prompt: 'mcp.prompt.name'",
    )
    .await;

    harness.get_prompt("alpha_summarize").await.ok();

    let metadata = seen.metadata.lock().expect("lock").clone();
    assert_eq!(metadata.len(), 2, "request then response");
    assert_eq!(
        metadata[1],
        vec![("prompt".to_string(), "summarize".to_string())]
    );

    stop.cancel();
    harness.stop().await;
}

// ---------------------------------------------------------------------------
// `requestHeaderModifier` on the MCP upstream path
// ---------------------------------------------------------------------------

/// Boot a gateway with an HTTP MCP target, a processor, and a route header
/// modifier — the combination the templating exists for.
async fn with_modifier(
    processor_host: &str,
    methods: &str,
    modifier: &str,
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
              requestHeaderModifier:
{modifier}
              mcpGuardrails:
                processors:
                  - host: "{processor_host}"
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

/// Make one `tools/call` through the gateway, asserting it succeeded.
///
/// The assertion matters: if the call failed, the last request the target saw
/// would be the handshake or a listing, and every header assertion below would
/// be checking the wrong request.
async fn call_through(port: u16) {
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
            .any(|b| b.as_text().is_some_and(|t| t.text == "served")),
        "and should have reached the HTTP target"
    );

    let _ = client.cancel().await;
}

#[tokio::test]
async fn a_guardrails_value_reaches_the_upstream_as_a_header() {
    // The whole point: a processor classifies a call in-band, and the MCP
    // server behind the gateway is told, without speaking to the processor.
    let (target_port, target_headers, stop_target) = http_target().await;
    let (host, _, stop_proc) = processor(Script::Annotate).await;
    let (port, shutdown) = with_modifier(
        &host,
        r#"{ "tools/call": request }"#,
        "                set:\n                  x-classification: '{{mcpGuardrails.classification}}'",
        target_port,
    )
    .await;

    call_through(port).await;

    let seen = target_headers.lock().expect("lock").clone();
    let last = seen.last().expect("the target should have been called");
    assert_eq!(
        last.iter()
            .find(|(k, _)| k == "x-classification")
            .map(|(_, v)| v.clone()),
        Some("phishing".to_string()),
        "saw {last:?}"
    );

    // Per call, not per connection: the handshake ran before any processor.
    let handshake = seen.first().expect("there should be a handshake");
    assert!(!handshake.iter().any(|(k, _)| k == "x-classification"));

    shutdown.cancel();
    stop_proc.cancel();
    stop_target.cancel();
}

#[tokio::test]
async fn an_unresolved_template_sends_no_header_at_all() {
    // Rather than sending `{{mcpGuardrails.absent}}` upstream as though it
    // were data.
    let (target_port, target_headers, stop_target) = http_target().await;
    let (host, _, stop_proc) = processor(Script::Annotate).await;
    let (port, shutdown) = with_modifier(
        &host,
        r#"{ "tools/call": request }"#,
        "                set:\n                  x-missing: '{{mcpGuardrails.absent}}'\n                  x-static: 'always'",
        target_port,
    )
    .await;

    call_through(port).await;

    let seen = target_headers.lock().expect("lock").clone();
    let last = seen.last().expect("the target should have been called");
    assert!(!last.iter().any(|(k, _)| k == "x-missing"), "saw {last:?}");
    assert!(
        last.iter().any(|(k, v)| k == "x-static" && v == "always"),
        "a literal beside it still goes; saw {last:?}"
    );

    shutdown.cancel();
    stop_proc.cancel();
    stop_target.cancel();
}

#[tokio::test]
async fn a_modifier_applies_without_any_guardrail_at_all() {
    // `requestHeaderModifier` used to parse and do nothing on an MCP route.
    // A static header must work with no processor in the picture.
    let (target_port, target_headers, stop_target) = http_target().await;
    let (host, _, stop_proc) = processor(Script::Pass).await;
    let (port, shutdown) = with_modifier(
        &host,
        r#"{ "prompts/get": request }"#,
        "                set:\n                  x-gateway: 'rusty'",
        target_port,
    )
    .await;

    call_through(port).await;

    let seen = target_headers.lock().expect("lock").clone();
    let last = seen.last().expect("the target should have been called");
    assert!(
        last.iter().any(|(k, v)| k == "x-gateway" && v == "rusty"),
        "no processor is keyed on tools/call, and the header still goes: {last:?}"
    );

    shutdown.cancel();
    stop_proc.cancel();
    stop_target.cancel();
}

#[tokio::test]
async fn the_route_wins_over_a_processors_header_mutation() {
    let (target_port, target_headers, stop_target) = http_target().await;
    let (host, _, stop_proc) = processor(Script::SetHeaders).await;
    let (port, shutdown) = with_modifier(
        &host,
        r#"{ "tools/call": request }"#,
        "                set:\n                  x-user-id: 'from-config'",
        target_port,
    )
    .await;

    call_through(port).await;

    let seen = target_headers.lock().expect("lock").clone();
    let last = seen.last().expect("the target should have been called");
    assert_eq!(
        last.iter()
            .find(|(k, _)| k == "x-user-id")
            .map(|(_, v)| v.clone()),
        Some("from-config".to_string()),
        "the processor asked for `u-42`; route configuration is the written \
         intent and runs last. Saw {last:?}"
    );

    shutdown.cancel();
    stop_proc.cancel();
    stop_target.cancel();
}

#[tokio::test]
async fn a_bad_template_fails_at_startup() {
    // Rather than a header that silently never resolves, which reads exactly
    // like a guardrail that never ran.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              requestHeaderModifier:
                set:
                  x-c: "{{{{jwt.sub}}}}"
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
        .expect("an unresolvable placeholder should be a startup failure");
    assert!(
        err.to_string().contains("mcpGuardrails.<key>"),
        "got: {err}"
    );
}

#[tokio::test]
async fn the_startup_warm_up_carries_the_routes_static_headers() {
    // The gateway lists tools once at startup to build its name index. An
    // upstream that requires a static header would reject that one request if
    // the modifier only applied to client calls.
    let (target_port, target_headers, stop_target) = http_target().await;
    let (host, _, stop_proc) = processor(Script::Pass).await;
    let (_port, shutdown) = with_modifier(
        &host,
        r#"{ "tools/call": request }"#,
        "                set:\n                  x-api-key: 'secret'\n                  x-classified: '{{mcpGuardrails.classification}}'",
        target_port,
    )
    .await;

    // Give the warm-up a moment; it runs as the federation comes up.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let seen = target_headers.lock().expect("lock").clone();
    assert!(
        seen.iter()
            .any(|req| req.iter().any(|(k, v)| k == "x-api-key" && v == "secret")),
        "no warm-up request carried the static header: {seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|req| req.iter().any(|(k, _)| k == "x-classified")),
        "and a template finds nothing to resolve against on a warm-up: {seen:?}"
    );

    shutdown.cancel();
    stop_proc.cancel();
    stop_target.cancel();
}
