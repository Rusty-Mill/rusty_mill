//! What the tool modules need from the MCP stack: parameter and result
//! wrappers, the error type, and [`register`], which turns one tool method
//! into a `rusty_mcp_server` tool whose input schema comes from the
//! argument type (`schemars`), so each argument struct stays the single
//! source of truth for both parsing and the advertised schema.
//!
//! The server's handlers block; each call drives the tool's async method on
//! the tokio runtime it was registered with, from a thread that is not one of
//! that runtime's workers (the HTTP and stdio transports use their own).

use std::future::Future;

use rusty_mcp_server::ServerBuilder;
use rusty_mcp_server::json::Value as WireValue;
use rusty_mcp_server::proto::{CallToolParams, CallToolResult, ContentBlock, ErrorCode, Tool};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::runtime::Handle;

use crate::server::HomelabServer;

pub use rusty_mcp_server::proto::ErrorData;

/// A tool's parsed arguments.
pub struct Parameters<T>(pub T);

/// A tool result carried as structured content (and as its JSON text).
pub struct Json<T>(pub T);

/// A protocol-level failure of a call: the request was wrong.
pub struct ToolError(ErrorData);

impl ToolError {
    /// The caller asked for something that cannot be done (bad arguments, a
    /// backend that is not configured).
    pub fn invalid(message: impl Into<String>) -> Self {
        Self(ErrorData::new(ErrorCode::INVALID_PARAMS, message))
    }
}

impl ToolError {
    /// The backend failed while carrying out a valid request.
    pub fn failed(message: impl Into<String>) -> Self {
        Self(ErrorData::new(ErrorCode::INTERNAL_ERROR, message))
    }
}

impl From<ToolError> for ErrorData {
    fn from(error: ToolError) -> Self {
        error.0
    }
}

/// How a tool's return value becomes a `tools/call` result.
pub trait ToolOutput {
    /// The `outputSchema` to advertise, if the result is structured.
    fn schema() -> Option<WireValue>;
    /// The result.
    fn into_result(self) -> Result<CallToolResult, ErrorData>;
}

impl ToolOutput for String {
    fn schema() -> Option<WireValue> {
        None
    }

    fn into_result(self) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult {
            content: vec![ContentBlock::text(self)],
            ..CallToolResult::default()
        })
    }
}

impl<T: Serialize + JsonSchema> ToolOutput for Json<T> {
    fn schema() -> Option<WireValue> {
        schema_of::<T>().ok()
    }

    fn into_result(self) -> Result<CallToolResult, ErrorData> {
        let internal = |why: String| ErrorData::new(ErrorCode::INTERNAL_ERROR, why);
        let text = serde_json::to_string(&self.0).map_err(|e| internal(e.to_string()))?;
        let structured = WireValue::from_json_str(&text).map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult {
            content: vec![ContentBlock::text(text)],
            structured_content: Some(structured),
            ..CallToolResult::default()
        })
    }
}

/// The JSON Schema of `T`, without the `$schema` and `title` members.
fn schema_of<T: JsonSchema>() -> Result<WireValue, String> {
    let mut schema = serde_json::to_value(schemars::schema_for!(T)).map_err(|e| e.to_string())?;
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    WireValue::from_json_str(&schema.to_string()).map_err(|e| e.to_string())
}

/// The call's arguments as `A` (an absent `arguments` is an empty object).
fn arguments<A: DeserializeOwned>(call: &CallToolParams) -> Result<A, ErrorData> {
    let text = call
        .arguments
        .as_ref()
        .map_or_else(|| "{}".to_owned(), WireValue::to_json_string);
    serde_json::from_str(&text)
        .map_err(|e| ErrorData::new(ErrorCode::INVALID_PARAMS, format!("invalid arguments: {e}")))
}

/// Offer `name` on `builder`: its arguments are parsed as `A`, `call` runs
/// the tool's async method on `rt`, and the result is shaped by `R`.
pub fn register<A, R, F, Fut>(
    builder: ServerBuilder,
    server: &HomelabServer,
    rt: &Handle,
    name: &'static str,
    description: &'static str,
    call: F,
) -> ServerBuilder
where
    A: DeserializeOwned + JsonSchema + 'static,
    R: ToolOutput,
    F: Fn(HomelabServer, A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, ErrorData>>,
{
    // A schema that cannot be built is a bug in the argument type; an empty
    // object schema keeps the tool callable and the tests catch the loss.
    let input = schema_of::<A>().unwrap_or_else(|_| WireValue::object());
    let mut tool = Tool::new(name, input);
    tool.description = Some(description.to_owned());
    tool.output_schema = R::schema();
    let (server, rt) = (server.clone(), rt.clone());
    builder.tool(tool, move |_ctx, params| {
        let args = arguments::<A>(&params)?;
        rt.block_on(call(server.clone(), args))?.into_result()
    })
}

/// A tool that takes no arguments.
#[derive(serde::Deserialize, JsonSchema)]
pub struct NoArgs {}
