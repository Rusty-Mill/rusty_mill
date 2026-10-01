//! [`McpToolset`] — consume an external MCP server's tools as ADK tools.
//!
//! This is the mirror of [`crate::McpServer`]: where that exposes Rust tools to
//! other ADK SDKs, this lets a Rust agent use tools from any MCP server, the
//! way ADK's `McpToolset` does in the other languages.

use adk_core::{AdkError, Args, FunctionDeclaration, InvocationContext, Result, Schema};
use adk_tools::{SharedTool, Tool, ToolContext, Toolset};
use async_trait::async_trait;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, Tool as McpToolEntry,
};
use rmcp::service::{Peer, RunningService, ServiceError};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::future::Future;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::line_cap::{cap_error, LineCapped};
use crate::protocol::{protocol_version, uppercase_types};

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
}

/// A live MCP session with a server subprocess, run by `rmcp`.
struct StdioConnection {
    service: RunningService<RoleClient, ClientInfo>,
    /// Killed when the connection is dropped (`kill_on_drop`).
    child: Child,
    handle: Handle,
}

/// What a request needs from its connection, owned so a request future
/// does not borrow the connection slot.
#[derive(Clone)]
struct Handle {
    peer: Peer<RoleClient>,
    timeout: Duration,
    /// Set when the server sent a line past the 16 MiB cap.
    exceeded: Arc<AtomicBool>,
}

impl Handle {
    /// Runs one request under the connection's deadline.
    async fn request<T>(
        &self,
        method: &str,
        request: impl Future<Output = std::result::Result<T, ServiceError>>,
    ) -> Result<T> {
        deadline(method, self.timeout, &self.exceeded, async {
            request.await.map_err(|e| request_error(method, e))
        })
        .await
    }
}

impl StdioConnection {
    /// Launches the server and completes the MCP handshake, all within
    /// `timeout`. A server that stalls or fails the handshake is killed
    /// rather than leaked.
    async fn spawn(params: &ConnectionParams, timeout: Duration) -> Result<Self> {
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

        let exceeded = Arc::new(AtomicBool::new(false));
        let stdout = LineCapped::new(stdout, Arc::clone(&exceeded));
        let info = ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("rusty-adk", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(protocol_version());

        let service = match deadline("initialize", timeout, &exceeded, async {
            info.serve((stdout, stdin))
                .await
                .map_err(|e| AdkError::Other(format!("MCP handshake with '{command}' failed: {e}")))
        })
        .await
        {
            Ok(service) => service,
            Err(err) => {
                let _ = child.kill().await;
                return Err(err);
            }
        };

        let handle = Handle {
            peer: service.peer().clone(),
            timeout,
            exceeded,
        };
        Ok(Self {
            service,
            child,
            handle,
        })
    }

    async fn shutdown(mut self) {
        let _ = self.service.cancel().await;
        let _ = self.child.kill().await;
    }
}

/// Bounds `work` by `limit` (design review 3.7, N16), and reports a capped
/// line from the server as the cause when that is what ended it.
async fn deadline<T>(
    method: &str,
    limit: Duration,
    exceeded: &AtomicBool,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    let outcome = match timeout(limit, work).await {
        Ok(result) => result,
        Err(_) => Err(AdkError::Other(format!(
            "MCP server did not respond to '{method}' within {limit:?}"
        ))),
    };
    match outcome {
        Err(_) if exceeded.load(Ordering::Acquire) => Err(AdkError::Other(cap_error())),
        other => other,
    }
}

fn request_error(method: &str, err: ServiceError) -> AdkError {
    match err {
        ServiceError::McpError(data) => {
            AdkError::Other(format!("MCP error on {method}: {}", data.message))
        }
        other => AdkError::Other(format!("MCP request '{method}' failed: {other}")),
    }
}

/// Tools discovered from an external MCP server.
pub struct McpToolset {
    params: ConnectionParams,
    filter: Option<HashSet<String>>,
    timeout: Duration,
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
        F: FnOnce(Handle) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut guard = self.connection.lock().await;
        if guard.is_none() {
            *guard = Some(StdioConnection::spawn(&self.params, self.timeout).await?);
        }
        let handle = guard
            .as_ref()
            .map(|c| c.handle.clone())
            .ok_or_else(|| AdkError::Other("MCP connection unavailable".into()))?;
        let result = op(handle).await;
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
                |h| async move { h.request("tools/list", h.peer.list_all_tools()).await },
            )
            .await?;
        Ok(entries
            .into_iter()
            .filter(|entry| {
                !entry.name.is_empty()
                    && self
                        .filter
                        .as_ref()
                        .is_none_or(|names| names.contains(entry.name.as_ref()))
            })
            .map(|entry| Arc::new(McpTool::from_entry(entry)) as SharedTool)
            .collect())
    }

    /// Calls a tool on the connected server.
    async fn call(&self, name: &str, args: Args) -> Result<Value> {
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(args);
        let result = self
            .with_connection(
                |h| async move { h.request("tools/call", h.peer.call_tool(params)).await },
            )
            .await?;
        Ok(decode_tool_result(&serde_json::to_value(result)?))
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
        let schema = Value::Object(entry.input_schema.as_ref().clone());
        Self {
            name: entry.name.into_owned(),
            description: entry
                .description
                .map(|d| d.into_owned())
                .unwrap_or_default(),
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
