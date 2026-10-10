//! [`McpToolset`] — consume an external MCP server's tools as ADK tools.
//!
//! This is the mirror of [`crate::McpServer`]: where that exposes Rust tools to
//! other ADK SDKs, this lets a Rust agent use tools from any MCP server, the
//! way ADK's `McpToolset` does in the other languages. The connection is a
//! `rusty-mcp-client` one over the server's stdio.

use adk_core::{AdkError, Args, FunctionDeclaration, InvocationContext, Result, Schema};
use adk_tools::{SharedTool, Tool, ToolContext, Toolset};
use async_trait::async_trait;
use rusty_mcp_client::proto::{CallToolResult, ContentBlock, Tool as McpToolEntry};
use rusty_mcp_client::{McpClient, McpClientError, McpServerSpec, McpTransport};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::protocol::{from_wire, uppercase_types};

/// Default deadline for one MCP request/response round trip — including
/// the `initialize` handshake, `tools/list`, and every `tools/call` —
/// measured from the moment the request is sent until its response arrives.
///
/// A stalled or hostile subprocess would otherwise leave a request waiting
/// forever, wedging the toolset's single connection [`Mutex`] and blocking
/// every later call. 60s matches this workspace's other MCP-call timeout
/// convention (`nexus_ai::tools::mcp_bridge::MCP_CALL_TIMEOUT`): real MCP
/// tools (web fetches, code execution, …) routinely take tens of seconds, so
/// this exists to recover from a wedge rather than police tool latency.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// How to reach an MCP server.
#[derive(Debug, Clone)]
pub enum ConnectionParams {
    /// Launch a local subprocess and speak JSON-RPC over its pipes.
    Stdio {
        /// The executable to run.
        command: String,
        /// Its arguments.
        args: Vec<String>,
        /// Extra environment variables.
        env: Vec<(String, String)>,
    },
}

impl ConnectionParams {
    /// Builds stdio connection parameters.
    pub fn stdio<I, S>(command: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ConnectionParams::Stdio {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
        }
    }

    /// Adds an environment variable to a stdio connection.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let ConnectionParams::Stdio { env, .. } = &mut self;
        env.push((key.into(), value.into()));
        self
    }

    /// The `rusty-mcp-client` spec for these parameters.
    fn spec(&self) -> McpServerSpec {
        let ConnectionParams::Stdio { command, args, env } = self;
        McpServerSpec {
            transport: McpTransport::Stdio,
            command: command.clone(),
            args: args.clone(),
            env: env.iter().cloned().collect(),
            ..McpServerSpec::default()
        }
    }
}

/// A live MCP session with a server subprocess.
struct Connection {
    client: Arc<McpClient>,
}

impl Connection {
    /// Launches the server and completes the MCP handshake, all within
    /// `timeout` (which also bounds every later request). A server that
    /// stalls or fails the handshake is stopped rather than leaked.
    async fn spawn(params: &ConnectionParams, timeout: Duration) -> Result<Self> {
        let ConnectionParams::Stdio { command, .. } = params;
        match McpClient::connect_with("rusty-adk", &params.spec(), timeout).await {
            Ok(client) => Ok(Self {
                client: Arc::new(client),
            }),
            Err(McpClientError::Spawn { source, .. }) => Err(AdkError::Config(format!(
                "cannot launch MCP server '{command}': {source}"
            ))),
            Err(e) => Err(AdkError::Other(format!(
                "MCP handshake with '{command}' failed: {e}"
            ))),
        }
    }

    async fn shutdown(self) {
        // Anything else holding the client (a request that just failed) is
        // gone by now; if not, dropping it stops the server anyway.
        if let Ok(client) = Arc::try_unwrap(self.client) {
            let _ = client.shutdown().await;
        }
    }
}

/// Runs one request, naming `method` in its failure.
async fn request<T>(
    method: &str,
    call: impl Future<Output = std::result::Result<T, McpClientError>>,
) -> Result<T> {
    call.await
        .map_err(|e| AdkError::Other(format!("MCP request '{method}' failed: {e}")))
}

/// Tools discovered from an external MCP server.
pub struct McpToolset {
    params: ConnectionParams,
    filter: Option<HashSet<String>>,
    timeout: Duration,
    connection: Mutex<Option<Connection>>,
    cached: Mutex<Option<Vec<SharedTool>>>,
}

impl McpToolset {
    /// Builds a toolset backed by the server at `params`.
    ///
    /// The connection is opened lazily on first use, so constructing a toolset
    /// never blocks or fails.
    pub fn new(params: ConnectionParams) -> Self {
        Self {
            params,
            filter: None,
            timeout: DEFAULT_REQUEST_TIMEOUT,
            connection: Mutex::new(None),
            cached: Mutex::new(None),
        }
    }

    /// Exposes only the named tools from the server.
    pub fn with_filter<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.filter = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// Overrides the default per-request timeout ([`DEFAULT_REQUEST_TIMEOUT`],
    /// 60s) applied to the `initialize` handshake and every subsequent
    /// request against this server.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Wraps this toolset for registration with an agent.
    pub fn shared(self) -> Arc<dyn Toolset> {
        Arc::new(self)
    }

    /// Runs `op` against the connection, opening it first if needed.
    ///
    /// Any failure clears the slot: a wedged or dead connection must not
    /// linger, so the next call respawns instead of failing again against
    /// the same broken one.
    async fn with_connection<T, F, Fut>(&self, op: F) -> Result<T>
    where
        F: FnOnce(Arc<McpClient>) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut guard = self.connection.lock().await;
        if guard.is_none() {
            *guard = Some(Connection::spawn(&self.params, self.timeout).await?);
        }
        let client = guard
            .as_ref()
            .map(|c| Arc::clone(&c.client))
            .ok_or_else(|| AdkError::Other("MCP connection unavailable".into()))?;
        let result = op(client).await;
        if result.is_err() {
            if let Some(broken) = guard.take() {
                broken.shutdown().await;
            }
        }
        result
    }

    /// Lists the server's tools, adapting each into an ADK tool.
    async fn discover(&self) -> Result<Vec<SharedTool>> {
        let entries = self
            .with_connection(
                |client| async move { request("tools/list", client.list_tools()).await },
            )
            .await?;
        Ok(entries
            .into_iter()
            .filter(|entry| {
                !entry.name.is_empty()
                    && self
                        .filter
                        .as_ref()
                        .is_none_or(|names| names.contains(entry.name.as_str()))
            })
            .map(|entry| Arc::new(McpTool::from_entry(entry)) as SharedTool)
            .collect())
    }

    /// Calls a tool on the connected server.
    async fn call(&self, name: &str, args: Args) -> Result<Value> {
        let result = self
            .with_connection(|client| async move {
                request("tools/call", client.call_tool(name, Some(args))).await
            })
            .await?;
        Ok(decode_tool_result(&result_json(&result)))
    }
}

/// An MCP tool result in the JSON shape [`decode_tool_result`] reads: its
/// text blocks and its error flag.
fn result_json(result: &CallToolResult) -> Value {
    let content: Vec<Value> = result
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(json!({"type": "text", "text": text})),
            _ => None,
        })
        .collect();
    json!({"content": content, "isError": result.is_error.unwrap_or(false)})
}

/// Converts an MCP tool result back into an ADK tool result.
///
/// MCP returns content blocks; ADK wants a JSON object. A text block holding
/// JSON is unwrapped, and anything else is carried through as text.
pub fn decode_tool_result(result: &Value) -> Value {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .next()
        })
        .unwrap_or("");

    let parsed = serde_json::from_str::<Value>(text).unwrap_or_else(|_| json!({"result": text}));

    if is_error {
        let message = parsed
            .get("error_message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| text.to_string());
        return adk_tools::error(message);
    }
    adk_core::wrap_tool_result(parsed)
}

/// A single tool proxied from an MCP server.
///
/// Holds only the declaration: the call is dispatched by the owning
/// [`McpToolset`], which owns the connection.
struct McpTool {
    name: String,
    description: String,
    parameters: Option<Schema>,
}

impl McpTool {
    fn from_entry(entry: McpToolEntry) -> Self {
        let schema = from_wire(&entry.input_schema);
        Self {
            name: entry.name,
            description: entry.description.unwrap_or_default(),
            parameters: serde_json::from_value::<Schema>(uppercase_types(&schema)).ok(),
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn declaration(&self) -> Option<FunctionDeclaration> {
        let mut declaration = FunctionDeclaration::new(&self.name, &self.description);
        declaration.parameters = self.parameters.clone();
        Some(declaration)
    }

    async fn run(&self, _args: Args, _ctx: &ToolContext) -> Result<Value> {
        Err(AdkError::tool(
            &self.name,
            "an MCP tool must be dispatched through its McpToolset",
        ))
    }
}

/// A tool bound to the toolset that owns its connection.
struct BoundMcpTool {
    inner: SharedTool,
    toolset: Arc<McpToolset>,
}

#[async_trait]
impl Tool for BoundMcpTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn declaration(&self) -> Option<FunctionDeclaration> {
        self.inner.declaration()
    }

    async fn run(&self, args: Args, _ctx: &ToolContext) -> Result<Value> {
        self.toolset.call(self.inner.name(), args).await
    }
}

#[async_trait]
impl Toolset for McpToolset {
    async fn tools(&self, _ctx: &InvocationContext) -> Result<Vec<SharedTool>> {
        // The tool list is fetched once and reused: an MCP server's tool set is
        // fixed for the life of the connection.
        let mut cache = self.cached.lock().await;
        if let Some(tools) = cache.as_ref() {
            return Ok(tools.clone());
        }
        let discovered = self.discover().await?;
        *cache = Some(discovered.clone());
        Ok(discovered)
    }

    async fn close(&self) -> Result<()> {
        if let Some(connection) = self.connection.lock().await.take() {
            connection.shutdown().await;
        }
        Ok(())
    }
}

/// Binds a discovered toolset so its tools dispatch through it.
///
/// [`Toolset::tools`] hands back declarations; this wraps each one so calling
/// it routes back through the connection the toolset owns.
pub async fn connect(toolset: Arc<McpToolset>, ctx: &InvocationContext) -> Result<Vec<SharedTool>> {
    let discovered = toolset.tools(ctx).await?;
    Ok(discovered
        .into_iter()
        .map(|inner| {
            Arc::new(BoundMcpTool {
                inner,
                toolset: Arc::clone(&toolset),
            }) as SharedTool
        })
        .collect())
}

/// A toolset whose tools are already bound to their connection.
///
/// This is what an agent should hold: `tools()` returns callable tools rather
/// than bare declarations.
pub struct BoundMcpToolset {
    inner: Arc<McpToolset>,
}

impl BoundMcpToolset {
    /// Wraps a toolset so its tools dispatch through it.
    pub fn new(toolset: McpToolset) -> Self {
        Self {
            inner: Arc::new(toolset),
        }
    }

    /// Wraps this toolset for registration with an agent.
    pub fn shared(self) -> Arc<dyn Toolset> {
        Arc::new(self)
    }
}

#[async_trait]
impl Toolset for BoundMcpToolset {
    async fn tools(&self, ctx: &InvocationContext) -> Result<Vec<SharedTool>> {
        connect(Arc::clone(&self.inner), ctx).await
    }

    async fn close(&self) -> Result<()> {
        self.inner.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_text_block_is_unwrapped() {
        let result = json!({
            "content": [{"type": "text", "text": r#"{"status":"success","temp":20}"#}],
            "isError": false,
        });
        let decoded = decode_tool_result(&result);
        assert_eq!(decoded["status"], "success");
        assert_eq!(decoded["temp"], 20);
    }

    #[test]
    fn a_plain_text_block_becomes_a_result_field() {
        let result = json!({"content": [{"type": "text", "text": "just words"}]});
        assert_eq!(decode_tool_result(&result)["result"], "just words");
    }

    #[test]
    fn an_mcp_error_becomes_an_adk_error_result() {
        let result = json!({
            "content": [{"type": "text", "text": r#"{"error_message":"nope"}"#}],
            "isError": true,
        });
        let decoded = decode_tool_result(&result);
        assert_eq!(decoded["status"], "error");
        assert_eq!(decoded["error_message"], "nope");
    }

    #[test]
    fn schema_types_round_trip_back_to_upper_case() {
        let mcp_schema = json!({
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
        });
        let schema: Schema = serde_json::from_value(uppercase_types(&mcp_schema)).unwrap();
        assert_eq!(schema.schema_type, Some(adk_core::SchemaType::Object));
        assert_eq!(
            schema.properties["city"].schema_type,
            Some(adk_core::SchemaType::String)
        );
    }

    #[test]
    fn stdio_params_carry_env_vars() {
        let params = ConnectionParams::stdio("npx", ["-y", "server"]).with_env("KEY", "v");
        let ConnectionParams::Stdio { command, args, env } = params;
        assert_eq!(command, "npx");
        assert_eq!(args, vec!["-y", "server"]);
        assert_eq!(env, vec![("KEY".to_string(), "v".to_string())]);
    }
}
