#![allow(clippy::unwrap_used)]
//! The mount in a real `rusty_axum` app: routed at a path, behind a guard,
//! with the native MCP client over real sockets. The same cases as
//! `rusty_mcp_axum`'s tests, which it replaces.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_axum::routing::{any, get};
use rusty_axum::{serve, IntoResponse, Request, Router, ServeOptions, Server, StatusCode};
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_client_native::{
    Client, ClientConfig, HttpConfig as ClientHttp, HttpTransport, NoHandler,
};
use rusty_mcp_server::{HttpConfig, HttpHandler, Mount, Server as McpServer};
use rusty_tokio::io::TcpListener;
use rusty_tokio::runtime::Runtime;

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

fn mcp_server() -> McpServer {
    McpServer::builder("mounted", "1")
        .tool(Tool::new("quick", schema()), |_c, _p| Ok(text("quick")))
        // Slower than the handler's plain-JSON window, so it is streamed.
        .tool(Tool::new("slow", schema()), |_c, _p| {
            std::thread::sleep(Duration::from_millis(600));
            Ok(text("slow"))
        })
        .build()
        .unwrap()
}

fn handler(server: McpServer, path: &str) -> Arc<HttpHandler> {
    Arc::new(HttpHandler::new(
        Arc::new(server),
        HttpConfig {
            path: path.to_owned(),
            ..HttpConfig::default()
        },
    ))
}

/// A running app. The runtime outlives the server it drives.
struct App {
    server: Option<Server>,
    runtime: Runtime,
}

impl App {
    fn start(router: Router, options: ServeOptions) -> Self {
        let runtime = rusty_tokio::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();
        let server = runtime.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            serve(listener, None, options, router).unwrap()
        });
        App {
            server: Some(server),
            runtime,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.server.as_ref().unwrap().local_addr()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            self.runtime.block_on(server.shutdown());
        }
    }
}

/// The MCP route behind `x-key: s3cret`, as an application's own guard
/// would be, next to an unguarded `/health`.
fn guarded_app(options: ServeOptions) -> App {
    let mcp = Mount::new(handler(mcp_server(), "/api/mcp"));
    let guarded = move |req: Request| {
        let allowed = req.headers().get("x-key") == Some("s3cret");
        let mcp = mcp.clone();
        async move {
            if !allowed {
                return (StatusCode::UNAUTHORIZED, "no key").into_response();
            }
            mcp.call(req).await
        }
    };
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/mcp", any(guarded));
    App::start(router, options)
}

fn client(
    url: String,
    headers: &[(&str, &str)],
) -> Result<Client<HttpTransport, NoHandler>, String> {
    let mut config = ClientHttp::new(url);
    for (name, value) in headers {
        config
            .headers
            .push(((*name).to_owned(), (*value).to_owned()));
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

fn raw(addr: SocketAddr, request: &str) -> String {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn a_routed_mount_serves_calls_and_other_routes_still_work() {
    let app = guarded_app(ServeOptions::default());
    let url = format!("http://{}/api/mcp", app.addr());
    let mut c = client(url, &[("x-key", "s3cret")]).unwrap();
    assert_eq!(c.list_tools().unwrap().len(), 2);
    assert_eq!(first_text(&c.call_tool("quick", None).unwrap()), "quick");
    let health = raw(
        app.addr(),
        "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(health.ends_with("ok"), "{health}");
}

#[test]
fn the_guard_decides_who_gets_through() {
    let app = guarded_app(ServeOptions::default());
    let url = format!("http://{}/api/mcp", app.addr());
    assert!(
        client(url, &[]).is_err(),
        "a request without the key was served"
    );
}

#[test]
fn a_slow_call_is_streamed_to_the_end() {
    let app = guarded_app(ServeOptions::default());
    let url = format!("http://{}/api/mcp", app.addr());
    let mut c = client(url, &[("x-key", "s3cret")]).unwrap();
    let t = Instant::now();
    assert_eq!(first_text(&c.call_tool("slow", None).unwrap()), "slow");
    let took = t.elapsed();
    assert!(took >= Duration::from_millis(500), "{took:?}");
}

#[test]
fn a_body_over_the_servers_limit_is_refused() {
    let app = guarded_app(ServeOptions {
        max_body_bytes: 64,
        ..ServeOptions::default()
    });
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"ping","pad":"{}"}}"#,
        "x".repeat(200)
    );
    let out = raw(
        app.addr(),
        &format!(
            "POST /api/mcp HTTP/1.1\r\nHost: localhost\r\nx-key: s3cret\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    assert!(out.starts_with("HTTP/1.1 413"), "{out}");
}

#[test]
fn a_pinned_mount_serves_whatever_path_it_is_reached_on() {
    let mcp = Mount::new(handler(mcp_server(), "/mcp")).at("/mcp");
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .fallback(move |req: Request| mcp.call(req));
    let app = App::start(router, ServeOptions::default());
    let url = format!("http://{}/secret/abc123", app.addr());
    let mut c = client(url, &[]).unwrap();
    assert_eq!(first_text(&c.call_tool("quick", None).unwrap()), "quick");
}

#[test]
fn a_tool_sees_the_headers_and_the_principal_of_its_request() {
    let server = McpServer::builder("who", "1")
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
    let mcp = Mount::new(handler(server, "/mcp"));
    let as_alice = move |req: Request| {
        let mut principal = Value::object();
        principal.insert("sub", Value::from("alice"));
        mcp.call_as(req, Some(principal))
    };
    let app = App::start(
        Router::new().route("/mcp", any(as_alice)),
        ServeOptions::default(),
    );
    let url = format!("http://{}/mcp", app.addr());
    let mut c = client(url, &[("X-Trace", "t-9")]).unwrap();
    let result = c.call_tool("whoami", None).unwrap();
    assert_eq!(first_text(&result), "alice/t-9");
}
