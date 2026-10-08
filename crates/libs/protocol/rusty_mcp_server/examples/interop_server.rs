//! A small stdio server for `tests/interop.rs`, which drives it with the
//! `rmcp` client. `INTEROP_CANCEL_FILE`, when set, is created once a `wait`
//! call observes its cancellation.

use rusty_json::Value;
use rusty_mcp_proto::{CallToolResult, ContentBlock, ErrorCode, ErrorData, Tool};
use rusty_mcp_server::{serve_stdio, Server};
use std::sync::Arc;
use std::time::Duration;

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

fn main() -> std::io::Result<()> {
    let cancel_file = std::env::var_os("INTEROP_CANCEL_FILE");
    let server = Server::builder("interop", "0.1.0")
        .instructions("a test server")
        .page_size(3)
        .tool(Tool::new("add", schema()), |_ctx, call| {
            let n = |k: &str| {
                call.arguments
                    .as_ref()
                    .and_then(|a| a.get(k))
                    .and_then(Value::as_i64)
            };
            match (n("a"), n("b")) {
                (Some(a), Some(b)) => Ok(text(&(a + b).to_string())),
                _ => Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "a and b are required",
                )),
            }
        })
        .tool(Tool::new("fail", schema()), |_ctx, _call| {
            Ok(CallToolResult {
                is_error: Some(true),
                ..text("it broke")
            })
        })
        .tool(Tool::new("progress", schema()), |ctx, _call| {
            for i in 1..=3 {
                ctx.progress(f64::from(i), Some(3.0), Some("step"));
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(text("finished"))
        })
        .tool(Tool::new("wait", schema()), move |ctx, _call| {
            while !ctx.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            if let Some(path) = &cancel_file {
                let _ = std::fs::write(path, "cancelled");
            }
            Ok(text("stopped"))
        })
        .build()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    serve_stdio(Arc::new(server))
}
