//! The demo server: seven tools, two prompts, two resources, a template and
//! completions, built on `rusty_mcp_server`. `tests/acceptance.rs` is its
//! acceptance suite.

use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{
    CallToolParams, CallToolResult, CompletionInfo, ContentBlock, ElicitAction, ElicitParams,
    ErrorCode, ErrorData, GetPromptResult, Prompt, PromptArgument, PromptMessage,
    ReadResourceResult, Reference, Resource, ResourceContents, ResourceTemplate, Role, Tool,
};
use rusty_mcp_server::{
    Ask, BuildError, CallContext, ChangeBroadcaster, ChangeKinds, Server, TaskContext, ToolOutcome,
    Turn, answer,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const TABLES: &[(&str, &[&str])] = &[
    ("users", &["id", "email", "created_at"]),
    ("orders", &["id", "user_id", "total_cents"]),
];
const DROP_TABLE: &str = "drop_table";
const SIGNING_KEY: &[u8] = b"rusty-mcp-demo-request-state-signing-key";

fn object(properties: &[(&str, &str)], required: &[&str]) -> Value {
    let mut props = Value::object();
    for (name, ty) in properties {
        let mut p = Value::object();
        p.insert("type", *ty);
        props.insert(*name, p);
    }
    let mut s = Value::object();
    s.insert("type", "object");
    s.insert("properties", props);
    s.insert(
        "required",
        Value::Array(required.iter().map(|r| Value::from(*r)).collect()),
    );
    s
}

fn tool(name: &str, description: &str, schema: Value) -> Tool {
    let mut t = Tool::new(name, schema);
    t.description = Some(description.to_owned());
    t
}

fn text(s: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(s)],
        ..CallToolResult::default()
    }
}

/// A result with structured content, and the same JSON as text for clients
/// that only read text.
fn structured(pairs: &[(&str, Value)]) -> CallToolResult {
    let mut v = Value::object();
    for (k, x) in pairs {
        v.insert(*k, x.clone());
    }
    CallToolResult {
        content: vec![ContentBlock::text(v.to_json_string())],
        structured_content: Some(v),
        ..CallToolResult::default()
    }
}

fn bad(why: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, why)
}

fn arg<'a>(call: &'a CallToolParams, key: &str) -> Option<&'a Value> {
    call.arguments.as_ref().and_then(|a| a.get(key))
}

fn int(call: &CallToolParams, key: &str) -> Result<i64, ErrorData> {
    arg(call, key)
        .and_then(Value::as_i64)
        .ok_or_else(|| bad(format!("`{key}` must be an integer")))
}

fn string(call: &CallToolParams, key: &str) -> Result<String, ErrorData> {
    arg(call, key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad(format!("`{key}` must be a string")))
}

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut pending = false;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if pending && !slug.is_empty() {
                slug.push('-');
            }
            pending = false;
            slug.extend(ch.to_lowercase());
        } else {
            pending = true;
        }
    }
    slug
}

fn count(n: usize) -> Value {
    Value::from(i64::try_from(n).unwrap_or(i64::MAX))
}

fn countdown(task: &TaskContext, call: CallToolParams) -> Result<CallToolResult, ErrorData> {
    let steps = u32::try_from(int(&call, "steps")?.clamp(0, 200)).unwrap_or(0);
    for remaining in (1..=steps).rev() {
        task.set_message(format!("{remaining} steps remaining"));
        let until = Instant::now() + Duration::from_millis(50);
        while Instant::now() < until {
            if task.is_cancelled() {
                return Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, "cancelled"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(text(format!("counted down {steps} steps")))
}

fn drop_table(ctx: &CallContext, call: CallToolParams) -> Result<ToolOutcome, ErrorData> {
    match ctx.turn(&call)? {
        Turn::Fresh => {
            let table = string(&call, "table")?;
            let form = ElicitParams::Form {
                message: format!("Really drop the `{table}` table? This cannot be undone."),
                requested_schema: object(&[("confirm", "boolean")], &["confirm"]),
                meta: None,
            };
            Ok(ToolOutcome::Ask(
                Ask::new()
                    .elicit("confirm-drop", &form)
                    .with_state(table.into_bytes()),
            ))
        }
        Turn::Resumed { state } => {
            // The table comes from the sealed state, not from the retry's
            // arguments: the user confirmed the one they saw.
            let table = String::from_utf8_lossy(state);
            let accepted = answer(&call, "confirm-drop")?.is_some_and(|r| {
                r.action == ElicitAction::Accept
                    && r.content
                        .as_ref()
                        .and_then(|c| c.get("confirm"))
                        .and_then(Value::as_bool)
                        == Some(true)
            });
            Ok(ToolOutcome::Done(text(if accepted {
                format!("dropped `{table}`")
            } else {
                format!("left `{table}` alone")
            })))
        }
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

fn prompt(name: &str, description: &str, args: Vec<PromptArgument>) -> Prompt {
    let mut p = Prompt::new(name);
    p.description = Some(description.to_owned());
    p.arguments = Some(args);
    p
}

fn user(text: String) -> GetPromptResult {
    GetPromptResult {
        messages: vec![PromptMessage {
            role: Role::User,
            content: ContentBlock::text(text),
        }],
        ..GetPromptResult::default()
    }
}

fn contents(uri: &str, mime: &str, body: String) -> ReadResourceResult {
    ReadResourceResult {
        contents: vec![ResourceContents::Text {
            uri: uri.to_owned(),
            mime_type: Some(mime.to_owned()),
            text: body,
            meta: None,
        }],
        ..ReadResourceResult::default()
    }
}

fn uptime() -> Duration {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// The demo server, with a fresh call counter and change broadcaster.
///
/// # Errors
/// A registration mistake, which is a bug in this file.
pub fn demo_server() -> Result<Server, BuildError> {
    let calls = Arc::new(AtomicU64::new(0));
    let changes = ChangeBroadcaster::new();
    let touched = changes.clone();
    let add_calls = Arc::clone(&calls);
    let divide_calls = Arc::clone(&calls);
    let two_ints = || object(&[("a", "integer"), ("b", "integer")], &["a", "b"]);
    let text_arg = || object(&[("text", "string")], &["text"]);

    Server::builder("rusty-mcp-demo", "0.5.0")
        .instructions(
            "Small arithmetic and text utilities, used to demonstrate the rusty-mcp \
             scaffold. Prefer `divide` over `add` when you need the remainder as well. \
             Resources expose configuration, uptime and table schemas; prompts cover \
             summarizing text and explaining errors.",
        )
        .page_size(3)
        .max_input_rounds(8)
        .state_key(SIGNING_KEY.to_vec())
        .notify_changes(&changes, ChangeKinds::all())
        .tool(
            tool("add", "Add two integers and return the sum.", two_ints()),
            move |_c, call| {
                let (a, b) = (int(&call, "a")?, int(&call, "b")?);
                let sum = a
                    .checked_add(b)
                    .ok_or_else(|| bad(format!("{a} + {b} overflows a 64-bit integer")))?;
                let n = add_calls.fetch_add(1, Ordering::Relaxed) + 1;
                Ok(structured(&[
                    ("sum", Value::from(sum)),
                    ("calls", Value::from(i64::try_from(n).unwrap_or(i64::MAX))),
                ]))
            },
        )
        .tool(
            tool(
                "divide",
                "Divide two integers, returning quotient and remainder.",
                two_ints(),
            ),
            move |_c, call| {
                let (a, b) = (int(&call, "a")?, int(&call, "b")?);
                if b == 0 {
                    return Err(bad("cannot divide by zero"));
                }
                let quotient = a
                    .checked_div(b)
                    .ok_or_else(|| bad(format!("{a} / {b} overflows a 64-bit integer")))?;
                divide_calls.fetch_add(1, Ordering::Relaxed);
                Ok(structured(&[
                    ("quotient", Value::from(quotient)),
                    ("remainder", Value::from(a % b)),
                ]))
            },
        )
        .tool(
            tool(
                "slugify",
                "Convert text into a lowercase, hyphen-separated slug.",
                text_arg(),
            ),
            |_c, call| Ok(text(slugify(&string(&call, "text")?))),
        )
        .tool(
            tool(
                "text_stats",
                "Count the words, characters and lines in some text.",
                text_arg(),
            ),
            |_c, call| {
                let t = string(&call, "text")?;
                let lines = if t.is_empty() { 0 } else { t.lines().count() };
                Ok(structured(&[
                    ("words", count(t.split_whitespace().count())),
                    ("characters", count(t.chars().count())),
                    ("lines", count(lines)),
                ]))
            },
        )
        .task_tool(
            tool(
                "countdown",
                "Count down in steps, slowly. Returns a task handle to clients that support the tasks extension.",
                object(&[("steps", "integer")], &["steps"]),
            ),
            countdown,
        )
        .interactive_tool(
            tool(
                DROP_TABLE,
                "Drop a demo table. Asks the user to confirm before acting.",
                object(&[("table", "string")], &["table"]),
            ),
            drop_table,
        )
        .tool(
            tool(
                "touch_resource",
                "Announce that a resource changed, notifying subscribed clients.",
                object(&[("uri", "string")], &["uri"]),
            ),
            move |_c, call| {
                let uri = string(&call, "uri")?;
                touched.resource_updated(uri.clone());
                touched.resources_changed();
                Ok(text(format!(
                    "announced {uri} to {} listener(s)",
                    touched.listeners()
                )))
            },
        )
        .prompt(
            prompt(
                "summarize",
                "Summarize text in a given number of sentences.",
                vec![argument("text", true), argument("sentences", false)],
            ),
            |_c, get| {
                let args = get.arguments.as_ref();
                let text = args
                    .and_then(|a| a.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let n = args
                    .and_then(|a| a.get("sentences"))
                    .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
                    .unwrap_or(3);
                Ok(user(format!(
                    "Summarize the following in about {n} sentences.\n\n{text}"
                )))
            },
        )
        .prompt(
            prompt(
                "explain-error",
                "Explain an error message and suggest fixes.",
                vec![argument("error", true), argument("language", false)],
            ),
            |_c, get| {
                let args = get.arguments.as_ref();
                let error = args
                    .and_then(|a| a.get("error"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let context = args
                    .and_then(|a| a.get("language"))
                    .and_then(Value::as_str)
                    .map(|l| format!(" This is {l} code."))
                    .unwrap_or_default();
                Ok(user(format!(
                    "Explain what this error means and how to fix it.{context}\n\n{error}"
                )))
            },
        )
        .resource(
            {
                let mut r = Resource::new("config://demo", "demo-config");
                r.title = Some("Demo configuration".to_owned());
                r.description = Some("Static configuration for the demo server.".to_owned());
                r.mime_type = Some("application/json".to_owned());
                r
            },
            |_c, read| {
                Ok(contents(
                    &read.uri,
                    "application/json",
                    r#"{"greeting":"hello","tools":["add","divide","slugify","text_stats","countdown"]}"#
                        .to_owned(),
                ))
            },
        )
        .resource(
            {
                let mut r = Resource::new("status://uptime", "uptime");
                r.description = Some("How long this process has been running.".to_owned());
                r.mime_type = Some("text/plain".to_owned());
                r
            },
            |_c, read| {
                Ok(contents(
                    &read.uri,
                    "text/plain",
                    format!("{} seconds", uptime().as_secs()),
                ))
            },
        )
        .resource_template(
            {
                let mut t = ResourceTemplate::new("db://tables/{table}", "table-schema");
                t.description = Some("Column names for a demo table.".to_owned());
                t.mime_type = Some("application/json".to_owned());
                t
            },
            |_c, vars, read| {
                let name = vars.get("table").unwrap_or_default();
                let columns = TABLES
                    .iter()
                    .find(|(t, _)| *t == name)
                    .map(|(_, c)| *c)
                    .ok_or_else(|| bad(format!("no such table `{name}`")))?;
                let mut body = Value::object();
                body.insert("table", name);
                body.insert(
                    "columns",
                    Value::Array(columns.iter().map(|c| Value::from(*c)).collect()),
                );
                Ok(contents(&read.uri, "application/json", body.to_json_string()))
            },
        )
        .completer(|_c, req| {
            let prefix = req.argument.value.to_lowercase();
            let pool: Vec<String> = match (&req.reference, req.argument.name.as_str()) {
                (Reference::Prompt { name, .. }, "language") if name == "explain-error" => {
                    ["rust", "python", "typescript", "go", "java"]
                        .map(String::from)
                        .to_vec()
                }
                (Reference::Resource { .. }, "table") => {
                    TABLES.iter().map(|(n, _)| (*n).to_owned()).collect()
                }
                _ => Vec::new(),
            };
            Ok(CompletionInfo {
                values: pool
                    .into_iter()
                    .filter(|v| v.starts_with(&prefix))
                    .collect(),
                total: None,
                has_more: None,
            })
        })
        .build()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::slugify;

    #[test]
    fn slugifies() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  leading and trailing  "), "leading-and-trailing");
        assert_eq!(slugify("multiple---separators"), "multiple-separators");
        assert_eq!(slugify("Ünïcödé Tëxt"), "ünïcödé-tëxt");
        assert_eq!(slugify("!!!"), "");
        assert_eq!(slugify(""), "");
    }
}
