//! A small MCP server over stdio, used to check this crate against an
//! independent client. Tools: `echo`, `add`, `fail`, `slow`.

use rusty_json::Value;
use rusty_mcp_proto::prompts::{GetPromptResult, PromptMessage, Role};
use rusty_mcp_proto::schema::Kind;
use rusty_mcp_proto::{
    CallToolResult, Completion, Prompt, PromptArgument, Reference, Resource, ResourceContents,
    ResourceTemplate, Schema, Tool,
};
use rusty_mcp_server::stdio::{serve_stdio, Options};
use rusty_mcp_server::{Dispatcher, Server, ToolError};
use std::time::{Duration, Instant};

fn text_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError(format!("{name} must be a string")))
}

fn main() -> std::io::Result<()> {
    let server = Server::new("rusty-mcp-echo", "0.1.0")
        .instructions("A fixture server for conformance tests.")
        .tool(
            Tool::new(
                "echo",
                "Return the text unchanged.",
                Schema::object()
                    .field("text", Kind::String, "Text to echo", true)
                    .build(),
            ),
            |_, args| Ok(CallToolResult::text(text_arg(args, "text")?)),
        )
        .tool(
            Tool::new(
                "add",
                "Add two integers.",
                Schema::object()
                    .field("a", Kind::Integer, "", true)
                    .field("b", Kind::Integer, "", true)
                    .build(),
            ),
            |_, args| {
                let get = |n: &str| {
                    args.get(n)
                        .and_then(Value::as_i64)
                        .ok_or_else(|| ToolError(format!("{n} must be an integer")))
                };
                Ok(CallToolResult::text((get("a")? + get("b")?).to_string()))
            },
        )
        .tool(
            Tool::new("fail", "Always fails.", Schema::object().build()),
            |_, _| Err(ToolError::from("this tool always fails")),
        )
        .tool(
            Tool::new(
                "slow",
                "Wait up to five seconds, stopping if cancelled.",
                Schema::object().build(),
            ),
            |ctx, _| {
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(5) {
                    if ctx.is_cancelled() {
                        return Ok(CallToolResult::text("cancelled"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(CallToolResult::text("finished"))
            },
        );
    serve_stdio(Dispatcher::new(with_content(server)), Options::default())
}

/// Resources, a template, a prompt and a completer for the acceptance tests.
fn with_content(server: Server) -> Server {
    let mut greet = Prompt::new("greet");
    greet.description = Some("Greet someone.".into());
    let mut name = PromptArgument::new("name");
    name.required = Some(true);
    greet.arguments.push(name);
    let mut greeting = Resource::new("mem://greeting", "greeting");
    greeting.mime_type = Some("text/plain".into());
    server
        .resource(greeting, |_, uri| {
            Ok(vec![ResourceContents::text(uri, "hello")])
        })
        .resource_template(ResourceTemplate::new("mem://notes/{id}", "notes"))
        .read_other(|_, uri| {
            uri.strip_prefix("mem://notes/")
                .map(|id| Ok(vec![ResourceContents::text(uri, format!("note {id}"))]))
        })
        .prompt(greet, |_, args| {
            let who = args.get("name").map_or("there", String::as_str);
            Ok(GetPromptResult {
                description: Some("A greeting".into()),
                messages: vec![PromptMessage::text(Role::User, format!("Hello, {who}!"))],
            })
        })
        .completer(|_, params| match &params.reference {
            Reference::Prompt { name } if name == "greet" => {
                let values: Vec<String> = ["alice", "alex", "bob"]
                    .iter()
                    .filter(|v| v.starts_with(params.argument.value.as_str()))
                    .map(|v| (*v).to_string())
                    .collect();
                Completion {
                    total: u32::try_from(values.len()).ok(),
                    has_more: Some(false),
                    values,
                }
            }
            _ => Completion::default(),
        })
}
