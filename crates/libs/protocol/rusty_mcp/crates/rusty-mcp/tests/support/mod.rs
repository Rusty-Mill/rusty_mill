//! A real `rusty_mcp_server` behind `rusty_mcp_axum`, on a real socket: the
//! stack these layers ship in, so each layer is tested where it runs.
#![allow(dead_code, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, ReadResourceResult, Resource, Tool};
use rusty_mcp_server::{ChangeBroadcaster, ChangeKinds, HttpConfig, HttpHandler, Server};

/// The path the MCP handler answers on.
pub const PATH: &str = "/mcp";

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

fn arg_u64(call: &rusty_mcp_server::proto::CallToolParams, key: &str) -> u64 {
    call.arguments
        .as_ref()
        .and_then(|a| a.get(key))
        .and_then(Value::as_i64)
        .map_or(0, |n| u64::try_from(n).unwrap_or(0))
}

/// A server with `echo`, a deliberately slow `sleep`, and `whoami` (the
/// `sub` of the caller's principal), announcing resource changes on `changes`.
pub fn server(changes: &ChangeBroadcaster) -> Server {
    Server::builder("fixture", "1")
        .tool(Tool::new("echo", schema()), |_c, _call| Ok(text("echo")))
        .tool(Tool::new("sleep", schema()), |_c, call| {
            std::thread::sleep(Duration::from_millis(arg_u64(&call, "ms")));
            Ok(text("done"))
        })
        .tool(Tool::new("whoami", schema()), |c, _call| {
            let sub = c
                .caller()
                .principal
                .as_ref()
                .and_then(|p| p.get("sub"))
                .and_then(Value::as_str)
                .unwrap_or("nobody")
                .to_owned();
            Ok(text(&sub))
        })
        .resource(Resource::new("mem://a", "a"), |_c, _r| {
            Ok(ReadResourceResult::default())
        })
        .notify_changes(changes, ChangeKinds::all_resources())
        .build()
        .unwrap()
}

/// The MCP router for `server`, with the caller's principal read from a
/// `VerifiedToken` that an earlier auth layer left in the request extensions.
pub fn mcp_router(changes: &ChangeBroadcaster) -> Router {
    let handler = Arc::new(HttpHandler::new(
        Arc::new(server(changes)),
        HttpConfig {
            path: PATH.to_owned(),
            ..HttpConfig::default()
        },
    ));
    rusty_mcp_axum::router_with_principal(handler, 1 << 20, |parts| {
        let token = parts.extensions.get::<rusty_mcp::auth::VerifiedToken>()?;
        let mut principal = Value::object();
        principal.insert("sub", token.subject.clone()?);
        Some(principal)
    })
}

/// Serve `app` (which nests [`mcp_router`] at [`PATH`]) and return where.
pub async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Serve [`mcp_router`] wrapped by `wrap` (layers go there), nested at
/// [`PATH`].
pub async fn spawn(wrap: impl FnOnce(Router) -> Router) -> (SocketAddr, ChangeBroadcaster) {
    let changes = ChangeBroadcaster::new();
    let app = Router::new().nest_service(PATH, wrap(mcp_router(&changes)));
    (serve(app).await, changes)
}

const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

/// A stateless `tools/call` body.
pub fn call(tool: &str, arguments: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{arguments},{META}}}}}"#
    )
}

/// A stateless `tools/list` body.
pub fn tools_list() -> String {
    format!(r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{{META}}}}}"#)
}

/// POST `body` (a stateless request for `method`, naming `tool` where the
/// method needs it) with an optional bearer token.
pub async fn post(
    addr: SocketAddr,
    method: &str,
    tool: Option<&str>,
    body: String,
    token: Option<&str>,
) -> reqwest::Response {
    let mut request = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{addr}{PATH}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method)
        .body(body);
    if let Some(tool) = tool {
        request = request.header("Mcp-Name", tool);
    }
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    request.send().await.expect("request reaches the server")
}
