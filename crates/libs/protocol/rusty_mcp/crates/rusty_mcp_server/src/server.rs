//! A [`Service`] built from registered tools.

use crate::{Context, Service};
use rusty_json::Value;
use rusty_mcp_proto::jsonrpc::code;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, ErrorObject, Implementation, ListParams, ListToolsResult,
    ServerCapabilities, Tool,
};

/// Why a tool failed. It becomes a tool result with `isError` set, which the
/// model can read, not a protocol error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError(pub String);

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        ToolError(message)
    }
}

impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        ToolError(message.to_string())
    }
}

type ToolFn = Box<dyn Fn(&Context, &Value) -> Result<CallToolResult, ToolError> + Send + Sync>;

/// A server assembled from tools.
pub struct Server {
    info: Implementation,
    instructions: Option<String>,
    tools: Vec<(Tool, ToolFn)>,
}

impl Server {
    /// A server with no tools.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Server {
            info: Implementation::new(name, version),
            instructions: None,
            tools: Vec::new(),
        }
    }

    /// Usage hints sent in the `initialize` result.
    pub fn instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = Some(text.into());
        self
    }

    /// Register a tool. `handler` gets the arguments object (`Null` when the
    /// caller sent none). Registering a name twice replaces the earlier tool.
    pub fn tool<F>(mut self, tool: Tool, handler: F) -> Self
    where
        F: Fn(&Context, &Value) -> Result<CallToolResult, ToolError> + Send + Sync + 'static,
    {
        self.tools.retain(|(t, _)| t.name != tool.name);
        self.tools.push((tool, Box::new(handler)));
        self
    }
}

impl Service for Server {
    fn info(&self) -> (Implementation, ServerCapabilities, Option<String>) {
        let capabilities = if self.tools.is_empty() {
            ServerCapabilities::default()
        } else {
            ServerCapabilities::tools_only()
        };
        (self.info.clone(), capabilities, self.instructions.clone())
    }

    fn list_tools(&self, _params: &ListParams) -> Result<ListToolsResult, ErrorObject> {
        Ok(ListToolsResult {
            tools: self.tools.iter().map(|(t, _)| t.clone()).collect(),
            next_cursor: None,
        })
    }

    fn call_tool(
        &self,
        ctx: &Context,
        params: &CallToolParams,
    ) -> Result<CallToolResult, ErrorObject> {
        let Some((_, handler)) = self.tools.iter().find(|(t, _)| t.name == params.name) else {
            return Err(ErrorObject::new(
                code::INVALID_PARAMS,
                format!("Unknown tool: {}", params.name),
            ));
        };
        let null = Value::Null;
        let arguments = params.arguments.as_ref().unwrap_or(&null);
        Ok(handler(ctx, arguments)
            .unwrap_or_else(|ToolError(message)| CallToolResult::error(message)))
    }
}
