//! `McpManager` — owns the connected MCP clients, namespaces their tools, and
//! registers them into the `feed::ToolRegistry` as `McpToolFn` adapters. Tool
//! enumeration is cached at connect time so registration into the (sync)
//! registry needs no await; `reconnect()` re-establishes a crashed transport.

use std::sync::Arc;

use rk_feed::ToolRegistry;

use crate::inspect::{DefaultInspector, ReturnInspector};
use crate::tool::{McpToolDescriptor, McpToolFn};
use crate::{namespaced, McpClient, McpError};

struct ServerHandle {
    name: String,
    client: Arc<dyn McpClient>,
    tools: Vec<McpToolDescriptor>,
}

/// Holds every connected MCP server and registers their tools.
pub struct McpManager {
    servers: Vec<ServerHandle>,
    inspector: Arc<dyn ReturnInspector>,
}

impl McpManager {
    /// Empty manager with the default return inspector.
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
            inspector: Arc::new(DefaultInspector),
        }
    }

    /// Override the tool-return inspector.
    pub fn with_inspector(mut self, inspector: Arc<dyn ReturnInspector>) -> Self {
        self.inspector = inspector;
        self
    }

    /// Connect every server declared in `config`, returning a populated manager.
    /// Per the PRD trust note, a server that fails to start or enumerate is
    /// logged to stderr and skipped — never fatal — so one bad server does not
    /// block the session. An empty config yields an empty manager.
    ///
    /// Requires the `rmcp` transport feature (the real stdio/SSE clients);
    /// without it there is no transport to connect through.
    #[cfg(feature = "rmcp")]
    pub async fn from_config(config: &crate::McpConfig) -> Self {
        let mut manager = Self::new();
        for spec in &config.servers {
            match crate::transport::client_from_spec(spec).await {
                Ok(client) => {
                    if let Err(e) = manager.connect(&spec.name, client).await {
                        eprintln!(
                            "warning: MCP server '{}' tool enumeration failed, skipping: {e}",
                            spec.name
                        );
                    }
                }
                Err(e) => eprintln!(
                    "warning: MCP server '{}' failed to start, skipping: {e}",
                    spec.name
                ),
            }
        }
        manager
    }

    /// Connect `client` as server `name`: enumerate + cache its (namespaced)
    /// tools. A failure to enumerate skips the server (PRD: a server that fails
    /// to start is logged + skipped, never fatal).
    pub async fn connect(
        &mut self,
        name: &str,
        client: Arc<dyn McpClient>,
    ) -> Result<(), McpError> {
        let infos = client.list_tools().await?;
        let tools = infos
            .into_iter()
            .map(|i| McpToolDescriptor {
                name: namespaced(name, &i.name),
                remote_name: i.name,
                schema: i.schema,
            })
            .collect();
        self.servers.push(ServerHandle {
            name: name.to_string(),
            client,
            tools,
        });
        Ok(())
    }

    /// Whether any servers are connected.
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Register every connected server's tools into `registry` (sync — uses the
    /// cached descriptors). Each tool is a policy-vetted `McpToolFn`.
    pub fn register(&self, registry: &mut ToolRegistry) {
        for s in &self.servers {
            for d in &s.tools {
                registry.insert(Box::new(McpToolFn::new(
                    s.client.clone(),
                    d.clone(),
                    self.inspector.clone(),
                )));
            }
        }
    }

    /// Reconnect every server's transport (after a crash) and re-enumerate.
    /// Re-enumeration is **not** re-vetting (PRD trust note); policy still gates
    /// each call.
    pub async fn reconnect(&mut self) -> Result<(), McpError> {
        for s in &mut self.servers {
            s.client.reconnect().await?;
            let infos = s.client.list_tools().await?;
            s.tools = infos
                .into_iter()
                .map(|i| McpToolDescriptor {
                    name: namespaced(&s.name, &i.name),
                    remote_name: i.name,
                    schema: i.schema,
                })
                .collect();
        }
        Ok(())
    }

    /// `(server, tool_count)` pairs for `/mcp`.
    pub fn summary(&self) -> Vec<(String, usize)> {
        self.servers
            .iter()
            .map(|s| (s.name.clone(), s.tools.len()))
            .collect()
    }

    /// The namespaced tool names for one server (`/mcp <server>`).
    pub fn server_tools(&self, name: &str) -> Vec<String> {
        self.servers
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.tools.iter().map(|d| d.name.clone()).collect())
            .unwrap_or_default()
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
