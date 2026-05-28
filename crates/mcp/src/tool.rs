//! The `McpToolFn` adapter (PRD 07; ADR-0036 F15). Wraps a namespaced MCP tool
//! as a `feed::ToolFn` that returns a **structured `ToolOutcome`** — never a
//! `String` with a sniffed `ERROR:` prefix — so the ADR-0022 status contract
//! holds unbroken from built-ins through to MCP-sourced tools.

use std::sync::Arc;

use async_trait::async_trait;
use rk_feed::ToolFn;
use rk_observe::{ToolOutcome, ToolStatus};
use serde_json::Value;

use crate::{inspect::ReturnInspector, McpClient, McpError};

/// A namespaced MCP tool advertised to the model.
#[derive(Debug, Clone)]
pub struct McpToolDescriptor {
    /// Namespaced id (`mcp__<server>__<tool>`).
    pub name: String,
    /// The un-namespaced tool name to send to the server.
    pub remote_name: String,
    /// JSON schema advertised to the model.
    pub schema: Value,
}

/// Adapts one MCP tool into the tool registry.
pub struct McpToolFn {
    client: Arc<dyn McpClient>,
    descriptor: McpToolDescriptor,
    inspector: Arc<dyn ReturnInspector>,
}

impl McpToolFn {
    /// Build the adapter over `client`, gated by `inspector` for return checks.
    pub fn new(
        client: Arc<dyn McpClient>,
        descriptor: McpToolDescriptor,
        inspector: Arc<dyn ReturnInspector>,
    ) -> Self {
        Self {
            client,
            descriptor,
            inspector,
        }
    }
}

#[async_trait]
impl ToolFn for McpToolFn {
    fn name(&self) -> &str {
        &self.descriptor.name
    }

    fn schema(&self) -> Value {
        self.descriptor.schema.clone()
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        match self
            .client
            .call_tool(&self.descriptor.remote_name, args)
            .await
        {
            Ok(result) => {
                // Tool-return inspection seam (threat-model): a small classifier
                // vets the MCP return *before* it can enter the model's context.
                match self.inspector.inspect(&self.descriptor.name, &result) {
                    crate::inspect::Inspection::Allow => ToolOutcome::ok(result),
                    crate::inspect::Inspection::Quarantine(reason) => ToolOutcome::new(
                        ToolStatus::Blocked,
                        format!("MCP return quarantined: {reason}"),
                    ),
                }
            }
            Err(McpError::Timeout) => ToolOutcome::new(ToolStatus::Timeout, "MCP call timed out"),
            Err(e) => ToolOutcome::error(format!("MCP call failed: {e}")),
        }
    }
}
