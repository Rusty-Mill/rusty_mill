//! The MCP server: the role's tools, each answered by [`core::handle`].

use std::sync::Arc;

use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{BuildError, Server};

use crate::core::{self, tools, Role, Workspace, MAX_TIMEOUT, OUTPUT_CAP};

/// One argument of a tool: name, JSON type, what it is.
type Arg = (&'static str, &'static str, &'static str);

/// A server offering `role`'s tools over `ws`.
///
/// # Errors
/// The builder refused the tool set (a duplicate name; not expected).
pub fn build<W: Workspace + Send + Sync + 'static>(
    ws: W,
    role: Role,
) -> Result<Server, BuildError> {
    let ws = Arc::new(ws);
    let mut builder = Server::builder(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .instructions("Files and commands in this session's root. Paths are relative to it.");
    for name in tools(role) {
        let ws = Arc::clone(&ws);
        builder = builder.tool(describe(name), move |_ctx, call| {
            let answer = core::handle(ws.as_ref(), role, &call.name, call.arguments.as_ref());
            Ok(CallToolResult {
                content: vec![ContentBlock::text(answer.body.to_json_string())],
                is_error: Some(answer.is_error),
                ..CallToolResult::default()
            })
        });
    }
    builder.build()
}

fn describe(name: &str) -> Tool {
    let (description, args, required): (String, &[Arg], &[&str]) = match name {
        "read" => (
            format!("Read a file as text, at most {OUTPUT_CAP} bytes."),
            &[("path", "string", "File path, relative to the root.")],
            &["path"],
        ),
        "list" => (
            "List a directory, sorted by name; symlinks are not followed.".to_owned(),
            &[("path", "string", "Directory path; the root when left out.")],
            &[],
        ),
        "stat" => (
            "Say what a path is (file, dir, symlink, other) and its size.".to_owned(),
            &[("path", "string", "Path, relative to the root.")],
            &["path"],
        ),
        "write" => (
            format!(
                "Create or replace a file, at most {OUTPUT_CAP} bytes; its directory must exist."
            ),
            &[
                ("path", "string", "File path, relative to the root."),
                ("content", "string", "The whole new content."),
            ],
            &["path", "content"],
        ),
        _ => (
            format!(
                "Run a program (no shell) and return its exit status and output, each stream \
                 at most {OUTPUT_CAP} bytes. Killed after timeout_s (at most {}).",
                MAX_TIMEOUT.as_secs()
            ),
            &[
                ("argv", "array", "Program and arguments."),
                (
                    "cwd",
                    "string",
                    "Working directory; the root when left out.",
                ),
                (
                    "timeout_s",
                    "integer",
                    "Seconds before it is killed; 60 when left out.",
                ),
                ("stdin", "string", "Text written to its stdin."),
            ],
            &["argv"],
        ),
    };
    let mut tool = Tool::new(name, schema(args, required));
    tool.description = Some(description);
    tool
}

fn schema(args: &[Arg], required: &[&str]) -> Value {
    let mut props = Value::object();
    for (name, ty, description) in args {
        let mut prop = Value::object();
        prop.insert("type", *ty);
        prop.insert("description", *description);
        if *ty == "array" {
            let mut items = Value::object();
            items.insert("type", "string");
            prop.insert("items", items);
        }
        props.insert(*name, prop);
    }
    let mut names = Value::array();
    for name in required {
        names.push(*name);
    }
    let mut schema = Value::object();
    schema.insert("type", "object");
    schema.insert("properties", props);
    schema.insert("required", names);
    schema.insert("additionalProperties", false);
    schema
}
