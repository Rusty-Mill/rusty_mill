#![allow(dead_code, clippy::unwrap_used)]
//! The server the client tests talk to: tools that add, take their time,
//! report progress, ask the user, run as tasks; a prompt, a resource and a
//! template. Shared by the in-process, child-process-free and HTTP suites.

use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{
    CallToolResult, ContentBlock, ElicitAction, ElicitParams, ErrorData, Prompt, PromptMessage,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Role, Tool,
};
use rusty_mcp_client_native::proto::{ClientCapabilities, ElicitResult, Wire};
use rusty_mcp_client_native::{ClientConfig, Handler};
use rusty_mcp_server::{answer, Ask, Server, ServerBuilder, ToolOutcome, Turn};
use std::time::Duration;

pub const SECS: Duration = Duration::from_secs(5);

pub fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

pub fn text(s: &str) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(s)],
        ..CallToolResult::default()
    }
}

pub fn form(message: &str) -> ElicitParams {
    ElicitParams::Form {
        message: message.to_owned(),
        requested_schema: schema(),
        meta: None,
    }
}

pub fn first_text(r: &CallToolResult) -> String {
    match &r.content[0] {
        ContentBlock::Text { text, .. } => text.clone(),
        other => panic!("not text: {other:?}"),
    }
}

pub fn builder() -> ServerBuilder {
    Server::builder("fixture", "1.0")
        .page_size(2)
        .tool(Tool::new("add", schema()), |_c, call| {
            let n = |k: &str| {
                call.arguments
                    .as_ref()
                    .and_then(|a| a.get(k))
                    .and_then(Value::as_i64)
            };
            match (n("a"), n("b")) {
                (Some(a), Some(b)) => Ok(text(&(a + b).to_string())),
                _ => Err(ErrorData::new(
                    rusty_mcp_client_native::proto::ErrorCode::INVALID_PARAMS,
                    "a and b are required",
                )),
            }
        })
        .tool(Tool::new("slow", schema()), |_c, _p| {
            std::thread::sleep(Duration::from_millis(400));
            Ok(text("late"))
        })
        .tool(Tool::new("progress", schema()), |ctx, _p| {
            for i in 1..=3 {
                ctx.progress(f64::from(i), Some(3.0), None);
            }
            Ok(text("done"))
        })
        .interactive_tool(Tool::new("confirm", schema()), |ctx, call| {
            match ctx.turn(&call)? {
                Turn::Fresh => Ok(ToolOutcome::Ask(
                    Ask::new()
                        .elicit("ok", &form("Proceed?"))
                        .with_state("pending"),
                )),
                Turn::Resumed { state } => {
                    let yes =
                        answer(&call, "ok")?.is_some_and(|r| r.action == ElicitAction::Accept);
                    Ok(ToolOutcome::Done(text(&format!(
                        "{} {}",
                        String::from_utf8_lossy(state),
                        if yes { "accepted" } else { "declined" }
                    ))))
                }
            }
        })
        .interactive_tool(Tool::new("nag", schema()), |_ctx, _call| {
            Ok(ToolOutcome::Ask(
                Ask::new().elicit("ok", &form("Again?")).with_state("x"),
            ))
        })
        .task_tool(Tool::new("job", schema()), |_ctx, _p| Ok(text("job done")))
        .task_tool(Tool::new("ask_job", schema()), |ctx, _p| {
            let reply = ctx.elicit("name", &form("Name?")).map_err(|_| {
                ErrorData::new(
                    rusty_mcp_client_native::proto::ErrorCode::INTERNAL_ERROR,
                    "no",
                )
            })?;
            let who = reply
                .content
                .as_ref()
                .and_then(|c| c.get("n"))
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned();
            Ok(text(&format!("hello {who}")))
        })
        .prompt(Prompt::new("hi"), |_c, _g| {
            Ok(rusty_mcp_client_native::proto::GetPromptResult {
                messages: vec![PromptMessage {
                    role: Role::User,
                    content: ContentBlock::text("hi there"),
                }],
                ..Default::default()
            })
        })
        .resource(Resource::new("mem://a", "a"), |_c, read| {
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::Text {
                    uri: read.uri,
                    mime_type: None,
                    text: "contents of a".to_owned(),
                    meta: None,
                }],
                ..ReadResourceResult::default()
            })
        })
        .resource_template(
            ResourceTemplate::new("mem://t/{x}", "t"),
            |_c, vars, read| {
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::Text {
                        uri: read.uri,
                        mime_type: None,
                        text: format!("x={}", vars.get("x").unwrap_or_default()),
                        meta: None,
                    }],
                    ..ReadResourceResult::default()
                })
            },
        )
}

pub fn capabilities(json: &str) -> ClientCapabilities {
    ClientCapabilities::from_value(&Value::from_json_str(json).unwrap()).unwrap()
}

/// A client that can be asked questions.
pub fn eliciting() -> ClientConfig {
    let mut c = ClientConfig::new("test-client", "0.1");
    c.capabilities = capabilities(r#"{"elicitation":{"form":{}}}"#);
    c
}

/// A client that can be asked questions and run tasks.
pub fn with_tasks() -> ClientConfig {
    let mut c = ClientConfig::new("test-client", "0.1");
    c.capabilities = capabilities(
        r#"{"elicitation":{"form":{}},"extensions":{"io.modelcontextprotocol/tasks":{}}}"#,
    );
    c
}

/// Answers every question with "Ann" (or declines), and remembers them.
pub struct Asker {
    pub accept: bool,
    pub asked: Vec<String>,
}

impl Handler for Asker {
    fn elicit(&mut self, params: &ElicitParams) -> ElicitResult {
        if let ElicitParams::Form { message, .. } = params {
            self.asked.push(message.clone());
        }
        let mut content = Value::object();
        content.insert("n", "Ann");
        ElicitResult {
            action: if self.accept {
                ElicitAction::Accept
            } else {
                ElicitAction::Decline
            },
            content: self.accept.then_some(content),
            meta: None,
        }
    }
}
