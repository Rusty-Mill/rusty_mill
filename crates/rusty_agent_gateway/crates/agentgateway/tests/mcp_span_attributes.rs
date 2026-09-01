//! Proof that a guardrail's `metadata` bag leaves the process as span
//! attributes.
//!
//! Everything else about this feature can be tested by inspecting values in
//! memory, and is. What cannot is the part that matters to an operator: that
//! the values a processor returned actually arrive at a collector. So this
//! stands up a fake OTLP endpoint, runs a real call through a real gateway
//! with a real gRPC processor in front of it, flushes, and looks for the
//! values in the bytes that were sent.
//!
//! # One test, one binary
//!
//! Installing a `tracing` subscriber globally is once-per-process, and a
//! thread-local one would not reach the tokio worker threads the gateway
//! serves requests on. So this file holds exactly one test, and `cargo test`
//! gives it a binary of its own.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use agentgateway_mcp::{McpRequest, McpRequestResult, McpResponseResult, request_result};
use prost::Message as _;
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::StreamableHttpClientTransport};
use rusty_mcp::otel::OtelConfig;
use tokio_util::sync::CancellationToken;

/// Distinctive enough to find by scanning, and not a substring of anything
/// else the exporter sends.
const CLASSIFICATION: &str = "phishing-a7f3c1";
const RULE_ID: &str = "rule-9b2e";

/// Everything the fake collector was sent, concatenated.
type Received = Arc<Mutex<Vec<u8>>>;

/// A TCP listener that keeps whatever the OTLP exporter writes at it.
///
/// Decoding OTLP protobuf properly would be its own project. The values under
/// test are distinctive strings, and a string only reaches these bytes by
/// having been put on a span, so scanning is enough to prove what it needs to.
async fn collector(received: Received) -> SocketAddr {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("collector should bind");
    let addr = listener.local_addr().expect("should have an address");

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
                // A gRPC client waits for a reply; closing is enough to let the
                // exporter finish rather than hang on shutdown.
                let _ = socket.shutdown().await;
            });
        }
    });

    addr
}

/// A gRPC `ExtMcp` processor that passes every call and annotates it.
async fn processor() -> (String, CancellationToken) {
    use axum::body::Body;
    use http_body_util::BodyExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("processor should bind");
    let addr: SocketAddr = listener.local_addr().expect("should have an address");
    let shutdown = CancellationToken::new();
    let stopping = shutdown.clone();

    let app = axum::Router::new().fallback(axum::routing::any(
        |request: axum::extract::Request| async move {
            let is_request = request.uri().path().ends_with("CheckRequest");
            let body = request
                .into_body()
                .collect()
                .await
                .map(|b| b.to_bytes())
                .unwrap_or_default();
            let _ = McpRequest::decode(&body[5..]);

            let payload = if is_request {
                let bag = serde_json::json!({
                    "classification": CLASSIFICATION,
                    "rule": RULE_ID,
                    "score": 0.75,
                    "blocked": false,
                });
                McpRequestResult {
                    result: Some(request_result::Result::Pass(Default::default())),
                    metadata: match agentgateway_mcp::to_proto_value(bag).kind {
                        Some(prost_types::value::Kind::StructValue(s)) => Some(s),
                        _ => None,
                    },
                    ..Default::default()
                }
                .encode_to_vec()
            } else {
                McpResponseResult::default().encode_to_vec()
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
        },
    ));

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await;
    });

    (addr.to_string(), shutdown)
}

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("examples");
    // Windows names the built example `mock_mcp_server.exe`; `EXE_SUFFIX` is
    // empty everywhere else.
    path.push(format!("mock_mcp_server{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "fixture not built at {}", path.display());
    path.display().to_string()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guardrails_metadata_reaches_the_collector_as_span_attributes() {
    use tracing_subscriber::prelude::*;

    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let collector_addr = collector(Arc::clone(&received)).await;

    // `pipeline` rather than `init`, so this test brings its own subscriber
    // instead of racing for the process-wide one.
    let (guard, tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("span-attribute-test")
            .with_endpoint(format!("http://{collector_addr}"))
            .with_shutdown_timeout(Duration::from_secs(3)),
    )
    .expect("the pipeline should start");

    // Without a filter the exporter's own h2 traffic produces TRACE events,
    // which become spans, which are exported over h2 -- a feedback loop that
    // buries the one span this test is about.
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(tracing_subscriber::EnvFilter::new(
                "warn,agentgateway_mcp=info",
            )),
    )
    .expect("this binary holds one test, so nothing else has installed one");

    let (processor_host, stop_processor) = processor().await;

    let port = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind");
        listener.local_addr().expect("addr").port()
    };
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
                  - host: "{processor_host}"
                    timeout: 5s
                    methods: {{ "tools/call": request }}
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
                          MOCK_TOOLS: "echo"
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

    client
        .call_tool(CallToolRequestParams::new("alpha_echo".to_string()))
        .await
        .expect("the call should succeed");

    let _ = client.cancel().await;
    shutdown.cancel();
    stop_processor.cancel();

    // Spans are batched. Without this the collector sees nothing, which is the
    // most common way to end up staring at an empty collector while insisting
    // the code is instrumented.
    guard.shutdown();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let bytes = received.lock().expect("lock").clone();
    assert!(
        !bytes.is_empty(),
        "the collector received nothing at all; the span never left the process"
    );
    assert!(
        contains(&bytes, "tools/call"),
        "the request span should have been exported under the method name -- \
         `otel.name` renames it from the `tracing` name `mcp.request`"
    );
    assert!(
        contains(&bytes, "agentgateway_mcp::span"),
        "and it should be the span this crate opened, not something incidental"
    );
    assert!(
        contains(&bytes, "mcpGuardrails.classification"),
        "the processor's key should be on the span, namespaced by the policy"
    );
    assert!(
        contains(&bytes, CLASSIFICATION),
        "and carrying the value the processor actually returned"
    );
    assert!(
        contains(&bytes, "mcpGuardrails.rule") && contains(&bytes, RULE_ID),
        "every key in the bag, not just the first"
    );
    assert!(
        contains(&bytes, "mcpGuardrails.score") && contains(&bytes, "mcpGuardrails.blocked"),
        "including the non-string values, which take their own attribute types"
    );
}
