//! `rusty-mux-mcp`: the rusty_mux request catalog as MCP tools.
//!
//! Each tool is a method of [`rusty_mux_proto::schema::CATALOG`], with the
//! same name, summary and argument schema as `rusty-mux-ctl api describe`
//! prints, so the CLI and the tools cannot drift. A call runs exactly as
//! `rusty-mux-ctl METHOD --param=value ...` would, on a connection of its
//! own: id arguments take names, positions and id suffixes, and a missing
//! one defaults from `$RUSTY_MUX_BLOCK` and the like. The tool's text is
//! what ctl prints (a screen as plain text, a new entity's id), or `ok`
//! when it prints nothing; a refusal is a tool error carrying the reason.
//!
//! Because each call has its own connection, the methods that only make
//! sense within one (attaching, taking or giving back the lease, `sync`) are
//! not tools. `input` and `resize`, which need the lease, attach and take it
//! first in the same call, so they fail while someone else holds it.
//!
//! A call gives up on a server that stops answering at its deadline, or as
//! soon as the client cancels it. On Linux a stalled server therefore never
//! holds a call slot. Elsewhere connecting is not bounded, and on Windows
//! neither is a write to a server that stopped reading (see
//! [`rusty_mux_ctl::run_until`]).

use std::sync::Arc;
use std::time::Duration;

use rusty_mcp_server::json::{Map, Value};
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{BuildError, Server};
use rusty_mux_ctl::{Env, Exit, Stop};
use rusty_mux_proto::schema::{CATALOG, Method, Returns, Ty};

/// How a catalog method is offered, given one connection per call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Runs alone.
    Plain,
    /// Runs after attaching to its block and taking the lease.
    Leased,
    /// Means nothing on a connection that closes after one call: no tool.
    Connection,
}

fn shape(method: &Method) -> Shape {
    match method.name {
        "attach" | "detach" | "acquire-lease" | "release-lease" | "sync" => Shape::Connection,
        "input" | "resize" => Shape::Leased,
        _ => Shape::Plain,
    }
}

/// How long a call waits on the server by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The MCP server: one tool per catalog method that works on a connection
/// of its own, each run against the server `env` names and given up after
/// `timeout` or when the client cancels it, within the limits of
/// [`rusty_mux_ctl::run_until`].
pub fn server(env: Env, timeout: Duration) -> Result<Server, BuildError> {
    let env = Arc::new(env);
    let builder = Server::builder("rusty-mux", env!("CARGO_PKG_VERSION")).instructions(
        "Drives a rusty_mux terminal multiplexer: create sessions, windows and blocks, \
         read a block's screen, title, working directory and program status, and send \
         it input. Id arguments also take a name, a position or an id suffix.",
    );
    CATALOG
        .iter()
        .filter(|m| shape(m) != Shape::Connection)
        .fold(builder, |builder, method| {
            let env = Arc::clone(&env);
            builder.tool(tool(method), move |ctx, call| {
                let token = ctx.cancel_token();
                let stop = Stop::after(timeout).or_when(move || token.is_cancelled());
                Ok(run(method, &env, &stop, call.arguments.as_ref()))
            })
        })
        .build()
}

fn tool(method: &Method) -> Tool {
    Tool {
        description: Some(method.doc.to_owned()),
        annotations: Some(annotations(method)),
        ..Tool::new(method.name, rusty_mux_ctl::input_schema(method))
    }
}

/// Behaviour hints: reads are read-only and closing is destructive.
/// `input` types into a shell, which can delete data and reach any system,
/// so it is destructive and open-world; the rest stays on the local server.
fn annotations(method: &Method) -> Value {
    let read_only = matches!(method.returns, Returns::Text | Returns::Entries);
    let input = method.name == "input";
    let destructive = input || method.name.starts_with("close-");
    let hints: Map = [
        ("readOnlyHint", read_only),
        ("destructiveHint", destructive),
        ("openWorldHint", input),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), Value::Bool(v)))
    .collect();
    Value::Object(hints)
}

fn run(method: &Method, env: &Env, stop: &Stop, arguments: Option<&Value>) -> CallToolResult {
    let args = match argv(method, arguments) {
        Ok(args) => args,
        Err(reason) => return result(reason, true),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let exit = rusty_mux_ctl::run_until(&args, env, stop, &mut out, &mut err);
    match exit {
        Exit::Ok if out.is_empty() => result("ok".to_owned(), false),
        Exit::Ok => result(printed(&out), false),
        _ => result(printed(&err), true),
    }
}

/// What ctl printed, less the newline ending its last line: trailing
/// spaces and blank lines are part of a screen, title or path.
fn printed(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_suffix('\n').unwrap_or(&text).to_owned()
}

fn result(text: String, is_error: bool) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(text)],
        is_error: Some(is_error),
        ..CallToolResult::default()
    }
}

/// The `rusty-mux-ctl` command line for a call: the method with one
/// `--param=value` per argument, after attaching and taking the lease for a
/// [`Shape::Leased`] method. Arguments are checked against the
/// method's parameters here, so no argument can add a flag or a request.
fn argv(method: &Method, arguments: Option<&Value>) -> Result<Vec<String>, String> {
    let flags = flags(method, arguments)?;
    let mut args = Vec::new();
    if shape(method) == Shape::Leased {
        let block: Vec<String> = flags
            .iter()
            .filter(|f| f.starts_with("--block="))
            .cloned()
            .collect();
        for setup in ["attach", "acquire-lease"] {
            args.push(setup.to_owned());
            args.extend(block.iter().cloned());
            args.push("+".to_owned());
        }
    }
    args.push(method.name.to_owned());
    args.extend(flags);
    Ok(args)
}

fn flags(method: &Method, arguments: Option<&Value>) -> Result<Vec<String>, String> {
    let map = match arguments {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Object(map)) => map,
        Some(_) => return Err(format!("{}: the arguments must be an object", method.name)),
    };
    let mut flags = Vec::new();
    for (name, value) in map {
        let Some(param) = method.params.iter().find(|p| p.name == name) else {
            return Err(format!("{}: unknown parameter `{name}`", method.name));
        };
        let texts = match (param.ty, value) {
            (Ty::Words, Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| format!("{}: `{name}` must be a list of strings", method.name))?,
            _ => vec![scalar(value).ok_or_else(|| {
                format!(
                    "{}: `{name}` must be a string or a non-negative integer",
                    method.name
                )
            })?],
        };
        flags.extend(texts.iter().map(|text| format!("--{name}={text}")));
    }
    Ok(flags)
}

fn scalar(value: &Value) -> Option<String> {
    match (value.as_str(), value.as_u64()) {
        (Some(text), _) => Some(text.to_owned()),
        (None, Some(n)) => Some(n.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests;
