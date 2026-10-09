//! The Streamable HTTP endpoint: handler-level checks without a socket, then
//! an independent `rmcp` HTTP client against a real `rusty_serve` listener.

use rusty_http::{HeaderMap, Method, StatusCode};
use rusty_json::Value;
use rusty_mcp_proto::schema::Kind;
use rusty_mcp_proto::{CallToolResult, Message, Schema, Tool};
use rusty_mcp_server::http::McpHttp;
use rusty_mcp_server::{Dispatcher, Server, ToolError};
use rusty_serve::{Body, Handler, Request, Response};

fn dispatcher() -> Dispatcher<Server> {
    Dispatcher::new(
        Server::new("http-test", "1").tool(
            Tool::new(
                "echo",
                "Echo text.",
                Schema::object()
                    .field("text", Kind::String, "Text", true)
                    .build(),
            ),
            |_, args| {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError::from("text must be a string"))?;
                Ok(CallToolResult::text(text))
            },
        ),
    )
}

fn handler() -> McpHttp<Server> {
    McpHttp::new(dispatcher(), "/mcp")
}

/// Send one request through the handler and resolve a deferred body.
fn call(
    handler: &mut McpHttp<Server>,
    method: Method,
    target: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (StatusCode, String) {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let _ = map.insert(name, value);
    }
    let request = Request {
        method: &method,
        target,
        authorization: map.get("authorization"),
        if_match: None,
        headers: &map,
        body: body.as_bytes(),
    };
    let Response { status, body } = handler.handle(&request);
    match body {
        Body::Json(bytes) => (status, String::from_utf8(bytes).expect("utf8")),
        Body::Deferred(job) => {
            let (status, bytes) = job();
            (status, String::from_utf8(bytes).expect("utf8"))
        }
        Body::Stream { .. } => panic!("the stateless endpoint never streams"),
    }
}

const PING: &str = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

#[test]
fn a_request_gets_its_response_as_json() {
    let (status, body) = call(&mut handler(), Method::Post, "/mcp", &[], PING);
    assert_eq!(status, StatusCode::OK);
    assert!(
        matches!(Message::from_json(&body), Ok(Message::Response { .. })),
        "{body}"
    );
}

#[test]
fn a_tool_call_runs_through_the_endpoint() {
    let call_json = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"text":"over http"}}}"#;
    let (status, body) = call(
        &mut handler(),
        Method::Post,
        "/mcp",
        &[("accept", "application/json, text/event-stream")],
        call_json,
    );
    assert_eq!(status, StatusCode::OK);
    let Ok(Message::Response { result, .. }) = Message::from_json(&body) else {
        panic!("{body}");
    };
    assert_eq!(
        CallToolResult::from_value(&result).expect("valid"),
        CallToolResult::text("over http")
    );
}

#[test]
fn notifications_are_accepted_without_a_body() {
    let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let (status, body) = call(&mut handler(), Method::Post, "/mcp", &[], note);
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body.is_empty());
}

#[test]
fn wrong_path_method_and_accept_are_refused() {
    let mut h = handler();
    assert_eq!(
        call(&mut h, Method::Post, "/other", &[], PING).0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&mut h, Method::Post, "/mcp?x=1", &[], PING).0,
        StatusCode::OK
    );
    assert_eq!(
        call(&mut h, Method::Get, "/mcp", &[], "").0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        call(&mut h, Method::Delete, "/mcp", &[], "").0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        call(
            &mut h,
            Method::Post,
            "/mcp",
            &[("accept", "text/html")],
            PING
        )
        .0,
        StatusCode::NOT_ACCEPTABLE
    );
    assert_eq!(
        call(&mut h, Method::Post, "/mcp", &[("accept", "*/*")], PING).0,
        StatusCode::OK
    );
}

#[test]
fn bad_bodies_get_a_json_rpc_error_with_400() {
    let mut h = handler();
    let (status, body) = call(&mut h, Method::Post, "/mcp", &[], "not json");
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        matches!(Message::from_json(&body), Ok(Message::Error { .. })),
        "{body}"
    );
    assert_eq!(
        call(
            &mut h,
            Method::Post,
            "/mcp",
            &[],
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#
        )
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn protocol_version_header_is_checked() {
    let mut h = handler();
    assert_eq!(
        call(
            &mut h,
            Method::Post,
            "/mcp",
            &[("mcp-protocol-version", "2025-06-18")],
            PING
        )
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &mut h,
            Method::Post,
            "/mcp",
            &[("mcp-protocol-version", "1999-01-01")],
            PING
        )
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn a_browser_origin_is_refused_unless_allowed() {
    let mut h = McpHttp::new(dispatcher(), "/mcp").allow_origin("http://localhost:3000");
    assert_eq!(
        call(
            &mut h,
            Method::Post,
            "/mcp",
            &[("origin", "http://evil.example")],
            PING
        )
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &mut h,
            Method::Post,
            "/mcp",
            &[("origin", "http://localhost:3000")],
            PING
        )
        .0,
        StatusCode::OK
    );
    // No Origin header (a non-browser client) is fine.
    assert_eq!(
        call(&mut h, Method::Post, "/mcp", &[], PING).0,
        StatusCode::OK
    );
    let mut strict = handler();
    assert_eq!(
        call(
            &mut strict,
            Method::Post,
            "/mcp",
            &[("origin", "http://localhost:3000")],
            PING
        )
        .0,
        StatusCode::FORBIDDEN
    );
}

#[test]
fn host_allowlist_and_authorization_hooks() {
    let mut h = McpHttp::new(dispatcher(), "/mcp")
        .allowed_hosts(vec!["127.0.0.1:8080".into()])
        .authorize(|req| req.authorization == Some("Bearer s3cret"));
    let good = [
        ("host", "127.0.0.1:8080"),
        ("authorization", "Bearer s3cret"),
    ];
    assert_eq!(
        call(&mut h, Method::Post, "/mcp", &good, PING).0,
        StatusCode::OK
    );
    let bad_host = [("host", "evil.example"), ("authorization", "Bearer s3cret")];
    assert_eq!(
        call(&mut h, Method::Post, "/mcp", &bad_host, PING).0,
        StatusCode::FORBIDDEN
    );
    let no_auth = [("host", "127.0.0.1:8080")];
    assert_eq!(
        call(&mut h, Method::Post, "/mcp", &no_auth, PING).0,
        StatusCode::UNAUTHORIZED
    );
}

// ------------------------------------------------------- a real socket, rmcp client

#[tokio::test(flavor = "current_thread")]
async fn rmcp_http_client_talks_to_the_endpoint() {
    use rmcp::model::CallToolRequestParams;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::ServiceExt;

    let server = rusty_serve::Server::bind("127.0.0.1:0".parse().expect("addr"), handler())
        .expect("binds a loopback port");
    let addr = server.local_addr().expect("local addr");
    let stop = server.shutdown_handle().expect("shutdown handle");
    let worker = std::thread::spawn(move || server.run());

    let config = StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"));
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("initialize over HTTP");
    let info = client.peer_info().expect("server info");
    assert_eq!(
        info.server_info.as_ref().expect("serverInfo").name,
        "http-test"
    );

    let tools = client.list_all_tools().await.expect("tools/list");
    assert_eq!(tools[0].name, "echo");
    let mut args = serde_json::Map::new();
    args.insert("text".into(), "hello over http".into());
    let result = client
        .call_tool(CallToolRequestParams::new("echo").with_arguments(args))
        .await
        .expect("tools/call");
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    assert_eq!(text, "hello over http");

    let _ = client.cancel().await;
    stop.shutdown();
    worker.join().expect("server thread").expect("server ran");
}
