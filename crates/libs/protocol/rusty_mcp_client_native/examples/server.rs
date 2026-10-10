//! A tiny stdio server for `tests/child_process.rs`, which starts it as a
//! child process and talks to it with this crate's client.

use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, ErrorCode, ErrorData, Tool};
use rusty_mcp_server::{serve_stdio, Server};
use std::sync::Arc;

fn main() -> std::io::Result<()> {
    let mut schema = Value::object();
    schema.insert("type", "object");
    let server = Server::builder("child", "1")
        .tool(Tool::new("add", schema), |_ctx, call| {
            let n = |k: &str| {
                call.arguments
                    .as_ref()
                    .and_then(|a| a.get(k))
                    .and_then(Value::as_i64)
            };
            match (n("a"), n("b")) {
                (Some(a), Some(b)) => Ok(CallToolResult {
                    content: vec![ContentBlock::text((a + b).to_string())],
                    ..CallToolResult::default()
                }),
                _ => Err(ErrorData::new(ErrorCode::INVALID_PARAMS, "a and b")),
            }
        })
        .build()
        .map_err(std::io::Error::other)?;
    serve_stdio(Arc::new(server))
}
