#![allow(dead_code, clippy::unwrap_used)]
//! The server the interop tests drive with the `rmcp` client. Included by
//! `examples/interop_server.rs` (stdio) and `tests/http_interop.rs` (HTTP)
//! through `#[path]`, so both serve exactly the same thing.

use rusty_json::Value;
use rusty_mcp_proto::{
    CallToolResult, CompletionInfo, ContentBlock, ErrorCode, ErrorData, GetPromptResult, Prompt,
    PromptArgument, PromptMessage, ReadResourceResult, Reference, Resource, ResourceContents,
    ResourceTemplate, Role, Tool,
};
use rusty_mcp_server::Server;
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

fn contents(uri: &str, body: &str) -> ReadResourceResult {
    ReadResourceResult {
        contents: vec![ResourceContents::Text {
            uri: uri.to_owned(),
            mime_type: Some("text/plain".to_owned()),
            text: body.to_owned(),
            meta: None,
        }],
        ..ReadResourceResult::default()
    }
}

fn argument(name: &str, required: bool) -> PromptArgument {
    PromptArgument {
        name: name.to_owned(),
        title: None,
        description: None,
        required: required.then_some(true),
    }
}

/// Four tools (`add`, `fail`, `progress`, `wait`), two prompts, four
/// resources and two templates, three entries to a page. `on_cancel` runs
/// when a `wait` call sees its cancellation.
pub fn interop_server(on_cancel: impl Fn() + Send + Sync + 'static) -> Server {
    let mut greet = Prompt::new("greet");
    greet.arguments = Some(vec![argument("name", true), argument("tone", false)]);
    let mut builder = Server::builder("interop", "0.1.0")
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
            // Bounded, so a call nobody can cancel does not outlive the test.
            let until = std::time::Instant::now() + Duration::from_secs(20);
            while !ctx.is_cancelled() && std::time::Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            if ctx.is_cancelled() {
                on_cancel();
            }
            Ok(text("stopped"))
        })
        .prompt(greet, |_ctx, get| {
            let name = get
                .arguments
                .as_ref()
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(GetPromptResult {
                description: Some("a greeting".to_owned()),
                messages: vec![PromptMessage {
                    role: Role::User,
                    content: ContentBlock::text(format!("Hello, {name}!")),
                }],
                ..GetPromptResult::default()
            })
        })
        .prompt(Prompt::new("bye"), |_ctx, _get| {
            Ok(GetPromptResult {
                messages: vec![PromptMessage {
                    role: Role::Assistant,
                    content: ContentBlock::text("Goodbye."),
                }],
                ..GetPromptResult::default()
            })
        })
        .resource_template(
            ResourceTemplate::new("file:///{dir}/{name}", "file"),
            |_ctx, vars, read| {
                let body = format!("{}:{}", vars.get("dir").unwrap(), vars.get("name").unwrap());
                Ok(contents(&read.uri, &body))
            },
        )
        .resource_template(
            ResourceTemplate::new("log://{+path}", "log"),
            |_ctx, vars, read| Ok(contents(&read.uri, vars.get("path").unwrap())),
        )
        .completer(|_ctx, req| {
            let prefix = req.argument.value.to_lowercase();
            let pool: &[&str] = match &req.reference {
                Reference::Prompt { .. } => &["Ada", "Alan", "Grace"],
                Reference::Resource { .. } => &["src", "docs"],
            };
            Ok(CompletionInfo {
                values: pool
                    .iter()
                    .filter(|v| v.to_lowercase().starts_with(&prefix))
                    .map(|v| (*v).to_owned())
                    .collect(),
                total: None,
                has_more: None,
            })
        });
    for name in ["a", "b", "c", "d"] {
        let uri = format!("mem://{name}");
        let body = format!("contents of {name}");
        builder = builder.resource(Resource::new(uri, name), move |_ctx, read| {
            Ok(contents(&read.uri, &body))
        });
    }
    builder.build().unwrap()
}
