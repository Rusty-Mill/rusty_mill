#![allow(clippy::unwrap_used)]
//! The bridge in a real axum app: nested under a path, behind a layer, with
//! the native MCP client over real sockets.

use axum::http::{Request, StatusCode};
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::Router;
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_client_native::{
    Client, ClientConfig, HttpConfig as ClientHttp, HttpTransport, NoHandler,
};
use rusty_mcp_server::{HttpConfig, HttpHandler, Server};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

fn server() -> Server {
    Server::builder("mounted", "1")
        .tool(Tool::new("quick", schema()), |_c, _p| Ok(text("quick")))
        // Slower than the handler's plain-JSON window, so it is streamed.
        .tool(Tool::new("slow", schema()), |_c, _p| {
            std::thread::sleep(Duration::from_millis(600));
            Ok(text("slow"))
        })
        .build()
        .unwrap()
}

/// Requires `x-key: s3cret`, as an application's own auth layer would.
async fn needs_key(request: Request<axum::body::Body>, next: Next) -> Response {
    if request
        .headers()
        .get("x-key")
        .is_some_and(|v| v == "s3cret")
    {
        next.run(request).await
    } else {
        let mut r = Response::new(axum::body::Body::from("no key"));
        *r.status_mut() = StatusCode::UNAUTHORIZED;
        r
    }
}

async fn app(max_body: usize) -> SocketAddr {
    let handler = Arc::new(HttpHandler::new(
        Arc::new(server()),
        HttpConfig {
            path: "/api/mcp".to_owned(),
            ..HttpConfig::default()
        },
    ));
    let mcp = rusty_mcp_axum::router(handler, max_body).layer(from_fn(needs_key));
    let app = Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .nest_service("/api/mcp", mcp);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn client(addr: SocketAddr, key: Option<&str>) -> Result<Client<HttpTransport, NoHandler>, String> {
    let mut config = ClientHttp::new(format!("http://{addr}/api/mcp"));
    if let Some(key) = key {
        config.headers.push(("x-key".to_owned(), key.to_owned()));
    }
    let transport = HttpTransport::new(config).unwrap();
    Client::connect(
        transport,
        ClientConfig::new("t", "1"),
        NoHandler,
        Duration::from_secs(5),
    )
    .map_err(|e| e.to_string())
}

fn first_text(r: &CallToolResult) -> &str {
    match &r.content[0] {
        ContentBlock::Text { text, .. } => text,
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_nested_mount_serves_calls_and_other_routes_still_work() {
    let addr = app(1 << 20).await;
    tokio::task::spawn_blocking(move || {
        let mut c = client(addr, Some("s3cret")).unwrap();
        assert_eq!(c.list_tools().unwrap().len(), 2);
        assert_eq!(first_text(&c.call_tool("quick", None).unwrap()), "quick");
    })
    .await
    .unwrap();
    let health = std::net::TcpStream::connect(addr).map(|mut s| {
        use std::io::{Read, Write};
        s.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    });
    assert!(health.unwrap().ends_with("ok"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_layer_in_front_decides_who_gets_through() {
    let addr = app(1 << 20).await;
    let err = tokio::task::spawn_blocking(move || client(addr, None).err())
        .await
        .unwrap();
    assert!(err.is_some(), "a request without the key was served");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_call_is_streamed_to_the_end() {
    let addr = app(1 << 20).await;
    let took = tokio::task::spawn_blocking(move || {
        let mut c = client(addr, Some("s3cret")).unwrap();
        let t = Instant::now();
        assert_eq!(first_text(&c.call_tool("slow", None).unwrap()), "slow");
        t.elapsed()
    })
    .await
    .unwrap();
    assert!(took >= Duration::from_millis(500), "{took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_body_over_the_limit_is_refused() {
    let addr = app(64).await;
    let outcome = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let body = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"ping","pad":"{}"}}"#, "x".repeat(200));
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        write!(
            s,
            "POST /api/mcp HTTP/1.1\r\nHost: localhost\r\nx-key: s3cret\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    })
    .await
    .unwrap();
    assert!(outcome.starts_with("HTTP/1.1 413"), "{outcome}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pinned_mount_serves_whatever_path_it_is_reached_on() {
    let handler = Arc::new(HttpHandler::new(
        Arc::new(server()),
        HttpConfig {
            path: "/mcp".to_owned(),
            ..HttpConfig::default()
        },
    ));
    let app = Router::new().nest_service(
        "/secret",
        rusty_mcp_axum::router_at(handler, 1 << 20, "/mcp"),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    tokio::task::spawn_blocking(move || {
        let transport =
            HttpTransport::new(ClientHttp::new(format!("http://{addr}/secret/abc123"))).unwrap();
        let mut c = Client::connect(
            transport,
            ClientConfig::new("t", "1"),
            NoHandler,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(first_text(&c.call_tool("quick", None).unwrap()), "quick");
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_sees_the_headers_and_the_principal_of_its_request() {
    let server = Server::builder("who", "1")
        .tool(Tool::new("whoami", schema()), |ctx, _p| {
            let caller = ctx.caller();
            let who = caller
                .principal
                .as_ref()
                .and_then(|p| p.get("sub"))
                .and_then(Value::as_str)
                .unwrap_or("nobody");
            let trace = caller.header("X-Trace").unwrap_or("none");
            Ok(text(&format!("{who}/{trace}")))
        })
        .build()
        .unwrap();
    let handler = Arc::new(HttpHandler::new(Arc::new(server), HttpConfig::default()));
    let mcp = rusty_mcp_axum::router_with_principal(handler, 1 << 20, |parts| {
        parts.extensions.get::<Claims>().map(|c| {
            let mut v = Value::object();
            v.insert("sub", Value::from(c.0.as_str()));
            v
        })
    })
    .layer(from_fn(
        |mut request: Request<axum::body::Body>, next: Next| async move {
            request.extensions_mut().insert(Claims("alice".to_owned()));
            next.run(request).await
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().nest_service("/mcp", mcp))
            .await
            .unwrap()
    });
    let mut config = ClientHttp::new(format!("http://{addr}/mcp"));
    config
        .headers
        .push(("X-Trace".to_owned(), "t-9".to_owned()));
    let result = tokio::task::spawn_blocking(move || {
        let mut c = Client::connect(
            HttpTransport::new(config).unwrap(),
            ClientConfig::new("t", "1"),
            NoHandler,
            Duration::from_secs(5),
        )
        .unwrap();
        c.call_tool("whoami", None).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(first_text(&result), "alice/t-9");
}

#[derive(Clone)]
struct Claims(String);
