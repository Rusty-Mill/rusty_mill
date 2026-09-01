//! [`McpToolset`] — consume an external MCP server's tools as ADK tools.
//!
//! This is the mirror of [`crate::McpServer`]: where that exposes Rust tools to
//! other ADK SDKs, this lets a Rust agent use tools from any MCP server, the
//! way ADK's `McpToolset` does in the other languages.

use adk_core::{AdkError, Args, FunctionDeclaration, InvocationContext, Result, Schema};
use adk_tools::{SharedTool, Tool, ToolContext, Toolset};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::protocol::PROTOCOL_VERSION;

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
}

/// A live JSON-RPC connection to an MCP server subprocess.
struct StdioConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: i64,
}

impl StdioConnection {
    async fn spawn(params: &ConnectionParams) -> Result<Self> {
        let ConnectionParams::Stdio { command, args, env } = params;

        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Leave stderr attached so the server's diagnostics stay visible.
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for (key, value) in env {
            cmd.env(key, value);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| AdkError::Config(format!("cannot launch MCP server '{command}': {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AdkError::Other("MCP server stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AdkError::Other("MCP server stdout unavailable".into()))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
        })
    }

    /// Sends a request and waits for its response.
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let payload = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.write(&payload).await?;

        loop {
            let line = self
                .stdout
                .next_line()
                .await?
                .ok_or_else(|| AdkError::Other("MCP server closed the connection".into()))?;
            if line.trim().is_empty() {
                continue;
            }

            let message: Value = serde_json::from_str(&line)?;
            // Skip anything that is not the response we are waiting for:
            // servers may interleave notifications on the same stream.
            if message.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let text = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(AdkError::Other(format!("MCP error on {method}: {text}")));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Sends a notification, which expects no response.
    async fn notify(&mut self, method: &str) -> Result<()> {
        self.write(&json!({"jsonrpc": "2.0", "method": method}))
            .await
    }

    async fn write(&mut self, payload: &Value) -> Result<()> {
        let text = serde_json::to_string(payload)?;
        self.stdin.write_all(text.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        let _ = self.child.kill().await;
        Ok(())
    }
}

/// Tools discovered from an external MCP server.
pub struct McpToolset {
    params: ConnectionParams,
    filter: Option<HashSet<String>>,
    connection: Mutex<Option<StdioConnection>>,
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

    /// Wraps this toolset for registration with an agent.
    pub fn shared(self) -> Arc<dyn Toolset> {
        Arc::new(self)
    }

    /// Opens the connection and completes the MCP handshake if needed.
    async fn ensure_connected(&self) -> Result<()> {
        let mut guard = self.connection.lock().await;
        if guard.is_some() {
            return Ok(());
        }

        let mut connection = StdioConnection::spawn(&self.params).await?;
        connection
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "rusty-adk", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await?;
        connection.notify("notifications/initialized").await?;

        *guard = Some(connection);
        Ok(())
    }

    /// Lists the server's tools, adapting each into an ADK tool.
    async fn discover(&self) -> Result<Vec<SharedTool>> {
        self.ensure_connected().await?;

        let result = {
            let mut guard = self.connection.lock().await;
            let connection = guard
                .as_mut()
                .ok_or_else(|| AdkError::Other("MCP connection unavailable".into()))?;
            connection.request("tools/list", json!({})).await?
        };

        let entries = result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut tools: Vec<SharedTool> = Vec::new();
        for entry in entries {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            if let Some(filter) = &self.filter {
                if !filter.contains(&name) {
                    continue;
                }
            }
            let description = entry
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let parameters = entry
                .get("inputSchema")
                .map(uppercase_types)
                .and_then(|schema| serde_json::from_value::<Schema>(schema).ok());

            tools.push(Arc::new(McpTool {
                name,
                description,
                parameters,
            }) as SharedTool);
        }
        Ok(tools)
    }

    /// Calls a tool on the connected server.
    async fn call(&self, name: &str, args: Args) -> Result<Value> {
        self.ensure_connected().await?;
        let mut guard = self.connection.lock().await;
        let connection = guard
            .as_mut()
            .ok_or_else(|| AdkError::Other("MCP connection unavailable".into()))?;

        let result = connection
            .request(
                "tools/call",
                json!({"name": name, "arguments": Value::Object(args)}),
            )
            .await?;

        Ok(decode_tool_result(&result))
    }
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

/// Restores the upper-case type names ADK's [`Schema`] expects.
fn uppercase_types(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, val)| {
                    if key == "type" {
                        if let Some(name) = val.as_str() {
                            return (key.clone(), json!(name.to_uppercase()));
                        }
                    }
                    (key.clone(), uppercase_types(val))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(uppercase_types).collect()),
        other => other.clone(),
    }
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
        let mut guard = self.connection.lock().await;
        if let Some(connection) = guard.as_mut() {
            connection.shutdown().await?;
        }
        *guard = None;
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
