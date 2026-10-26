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

use std::sync::Arc;

use rusty_mcp_server::json::{Map, Value};
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, Tool};
use rusty_mcp_server::{BuildError, Server};
use rusty_mux_ctl::{Env, Exit};
use rusty_mux_proto::schema::{CATALOG, Method, Returns};

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

/// The MCP server: one tool per catalog method that works on a connection
/// of its own, each run against the server `env` names.
pub fn server(env: Env) -> Result<Server, BuildError> {
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
            builder.tool(tool(method), move |_ctx, call| {
                Ok(run(method, &env, call.arguments.as_ref()))
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

/// Behaviour hints: reads are read-only, closing is destructive, and
/// nothing reaches beyond the local server.
fn annotations(method: &Method) -> Value {
    let read_only = matches!(method.returns, Returns::Text | Returns::Entries);
    let destructive = method.name.starts_with("close-");
    let hints: Map = [
        ("readOnlyHint", read_only),
        ("destructiveHint", destructive),
        ("openWorldHint", false),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), Value::Bool(v)))
    .collect();
    Value::Object(hints)
}

fn run(method: &Method, env: &Env, arguments: Option<&Value>) -> CallToolResult {
    let args = match argv(method, arguments) {
        Ok(args) => args,
        Err(reason) => return result(reason, true),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let exit = rusty_mux_ctl::run(&args, env, &mut out, &mut err);
    match exit {
        Exit::Ok if out.is_empty() => result("ok".to_owned(), false),
        Exit::Ok => result(String::from_utf8_lossy(&out).trim_end().to_owned(), false),
        _ => result(String::from_utf8_lossy(&err).trim_end().to_owned(), true),
    }
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
    map.iter()
        .map(|(name, value)| {
            if !method.params.iter().any(|p| p.name == name) {
                return Err(format!("{}: unknown parameter `{name}`", method.name));
            }
            let text = match (value.as_str(), value.as_u64()) {
                (Some(text), _) => text.to_owned(),
                (None, Some(n)) => n.to_string(),
                (None, None) => {
                    return Err(format!(
                        "{}: `{name}` must be a string or a non-negative integer",
                        method.name
                    ));
                }
            };
            Ok(format!("--{name}={text}"))
        })
        .collect()
}

#[cfg(test)]
mod tests;
