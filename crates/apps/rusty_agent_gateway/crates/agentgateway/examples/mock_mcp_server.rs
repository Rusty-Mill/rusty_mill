//! A minimal stdio MCP server, used as a fixture by the federation tests.
//!
//! It exports two tools and echoes which server handled a call, so a test can
//! prove a request reached the *right* target rather than merely reaching one.
//! `MOCK_LABEL` names the instance; `MOCK_TOOLS` overrides the tool list, which
//! is how a test sets up a name collision between two targets. `MOCK_DELAY_MS`
//! makes tool calls slow, so the timeout and concurrency tests have something
//! real to be slow about. The delay is on `call_tool` only, leaving startup
//! and `tools/list` prompt, and it ends early when the caller cancels, so a
//! timed-out call does not keep this process (and the test's stderr) alive.
//!
//! `MOCK_PROMPTS` and `MOCK_RESOURCES` add prompts and resources. Both default
//! to empty, and a server with neither advertises neither capability — which
//! is what lets a test check that the gateway only claims what its targets can
//! actually do.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_mcp_server::json::Value;
use rusty_mcp_server::proto::{
    CallToolResult, ContentBlock, ErrorCode, ErrorData, GetPromptResult, Prompt, PromptMessage,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Role, Tool,
};
use rusty_mcp_server::{CancelToken, Server, serve_stdio};

/// A comma-separated list from the environment, empty when unset.
fn list(name: &str, default: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_owned())
        .split(',')
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Sleep for `delay`, waking early once `cancel` is raised.
fn nap(delay: Duration, cancel: &CancelToken) {
    const SLICE: Duration = Duration::from_millis(10);
    let deadline = Instant::now() + delay;
    while !cancel.is_cancelled() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(SLICE));
    }
}

/// The schema every tool advertises: an object with no declared properties.
fn object_schema() -> Value {
    let mut schema = Value::object();
    schema.insert("type", "object");
    schema.insert("properties", Value::object());
    schema
}

fn main() -> std::io::Result<()> {
    let label: Arc<str> = std::env::var("MOCK_LABEL")
        .unwrap_or_else(|_| "mock".into())
        .into();
    let delay = Duration::from_millis(
        std::env::var("MOCK_DELAY_MS")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0),
    );

    // Only what this instance actually has is registered, and the server
    // advertises a capability only for what is registered, so a test can
    // check the gateway's own advertisement follows its targets.
    let mut builder = Server::builder("mock-mcp-server", "0.1.0");
    for name in list("MOCK_TOOLS", "echo,ping") {
        let mut tool = Tool::new(name.clone(), object_schema());
        tool.description = Some(format!("{name} on {label}"));
        // The label is what lets a test tell which target served the call.
        let text = format!("{label}:{name}");
        builder = builder.tool(tool, move |ctx, _call| {
            // Each call runs on a thread of its own, so a sleep here is slow
            // only for this call; startup and `tools/list` stay prompt.
            nap(delay, &ctx.cancel_token());
            Ok(CallToolResult {
                content: vec![ContentBlock::text(text.clone())],
                ..CallToolResult::default()
            })
        });
    }
    for name in list("MOCK_PROMPTS", "") {
        let mut prompt = Prompt::new(name.clone());
        prompt.description = Some(format!("{name} on {label}"));
        let text = format!("{label}:{name}");
        builder = builder.prompt(prompt, move |_ctx, _get| {
            Ok(GetPromptResult {
                messages: vec![PromptMessage {
                    role: Role::User,
                    content: ContentBlock::text(text.clone()),
                }],
                ..GetPromptResult::default()
            })
        });
    }
    let resources = list("MOCK_RESOURCES", "");
    for uri in &resources {
        let text = format!("{label}:{uri}");
        builder = builder.resource(
            Resource::new(uri.clone(), uri.clone()),
            move |_ctx, read| {
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::Text {
                        // The server's own URI, never the federated one -- which
                        // is what makes the gateway's re-qualification
                        // observable.
                        uri: read.uri,
                        mime_type: Some("text/plain".into()),
                        text: text.clone(),
                        meta: None,
                    }],
                    ..ReadResourceResult::default()
                })
            },
        );
    }
    // One fixed template, so a test has something with a `uriTemplate` to
    // gate without needing another environment variable. It lists but does
    // not read: only the resources above can be read.
    if !resources.is_empty() {
        let template = ResourceTemplate::new(format!("{label}:{{id}}"), "by-id");
        builder = builder.resource_template(template, |_ctx, _vars, read| {
            Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("unknown resource `{}`", read.uri),
            ))
        });
    }

    let server = builder
        .build()
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    serve_stdio(Arc::new(server))
}
