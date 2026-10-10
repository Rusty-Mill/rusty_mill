#![allow(clippy::unwrap_used)]
//! The client against an independent server: `rmcp`'s Streamable HTTP
//! server, with and without sessions. First-party code on both ends proves
//! little about the protocol; this is the other end.

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::{Client, ClientConfig, HttpConfig, HttpTransport, NoHandler};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
struct Adder;

impl ServerHandler for Adder {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let schema = serde_json::json!({"type": "object"});
        let tool = Tool::new(
            "add",
            "Add two integers",
            Arc::new(schema.as_object().cloned().unwrap()),
        );
        Ok(ListToolsResult::with_all_items(vec![tool]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let n = |k: &str| {
            request
                .arguments
                .as_ref()
                .and_then(|a| a.get(k))
                .and_then(serde_json::Value::as_i64)
        };
        match (request.name.as_ref(), n("a"), n("b")) {
            ("add", Some(a), Some(b)) => {
                Ok(CallToolResult::success(vec![ContentBlock::text((a + b).to_string())]).into())
            }
            _ => Err(ErrorData::invalid_params("add needs a and b", None)),
        }
    }
}

/// Host `Adder` on a runtime of its own; returns the URL.
fn host(sessions: bool) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let router = if sessions {
                let svc = StreamableHttpService::new(
                    || Ok(Adder),
                    Arc::new(LocalSessionManager::default()),
                    StreamableHttpServerConfig::default(),
                );
                axum::Router::new().nest_service("/mcp", svc)
            } else {
                let svc = StreamableHttpService::new(
                    || Ok(Adder),
                    Arc::new(NeverSessionManager::default()),
                    StreamableHttpServerConfig::default(),
                );
                axum::Router::new().nest_service("/mcp", svc)
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
    });
    format!("http://{}/mcp", rx.recv().unwrap())
}

fn args() -> Option<Value> {
    let mut v = Value::object();
    v.insert("a", 40);
    v.insert("b", 2);
    Some(v)
}

fn client(url: &str, config: ClientConfig) -> Client<HttpTransport, NoHandler> {
    let t = HttpTransport::new(HttpConfig::new(url)).unwrap();
    Client::connect(t, config, NoHandler, Duration::from_secs(10)).unwrap()
}

#[test]
fn an_rmcp_server_with_sessions_works_after_the_discover_fallback() {
    let url = host(true);
    let mut c = client(&url, ClientConfig::new("c", "1"));
    assert_eq!(c.list_tools().unwrap()[0].name, "add");
    let r = c.call_tool("add", args()).unwrap();
    assert!(format!("{r:?}").contains("42"));
}

#[test]
fn an_rmcp_server_without_sessions_works() {
    let url = host(false);
    let mut c = client(&url, ClientConfig::new("c", "1"));
    assert_eq!(c.list_tools().unwrap()[0].name, "add");
    let r = c.call_tool("add", args()).unwrap();
    assert!(format!("{r:?}").contains("42"));
}
