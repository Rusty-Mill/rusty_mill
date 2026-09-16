//! The `agent` subagent tool (PRD 03; ADR-0017). Spawning a child `Session`
//! would create a `feed → app → feed` cycle, so `feed` defines the
//! [`SessionFactory`] seam; `app` implements it and injects it here. The tool
//! calls the trait, never the concrete `Session` type. Recursion is bounded by
//! the factory's depth guard (`AgentDepthPolicy`, `RUSTYKEYS_MAX_AGENT_DEPTH`).

use std::sync::Arc;

use async_trait::async_trait;
use rk_observe::ToolOutcome;
use serde_json::Value;

use crate::error::ToolError;
use crate::tool::{ToolFn, ToolRegistry};

/// Builds + runs a child session for a focused subtask (ADR-0017). Implemented
/// by `app`; `feed` only names the trait, breaking the dependency cycle.
#[async_trait]
pub trait SessionFactory: Send + Sync {
    /// Run `task` in a child session (optionally restricted to `tools`),
    /// returning its final reply. Enforces the depth bound internally.
    async fn spawn(&self, task: &str, tools: Option<Vec<String>>) -> Result<String, ToolError>;

    /// Run `task` in a child session seeded with a cognitive-frame system-prompt
    /// preamble (ADR-0032 divergent→converge). The default ignores the frame and
    /// delegates to [`Self::spawn`]; `app` overrides it to inject the preamble.
    async fn spawn_framed(&self, task: &str, _frame: &str) -> Result<String, ToolError> {
        self.spawn(task, None).await
    }
}

struct AgentTool {
    factory: Arc<dyn SessionFactory>,
}

#[async_trait]
impl ToolFn for AgentTool {
    fn name(&self) -> &str {
        "agent"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {"type": "string"},
                "tools": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["task"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        let Some(task) = args.get("task").and_then(Value::as_str) else {
            return ToolOutcome::error("agent: missing 'task'");
        };
        let tools = args.get("tools").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        });
        match self.factory.spawn(task, tools).await {
            Ok(reply) => ToolOutcome::ok(reply),
            Err(e) => crate::error::outcome_from_error(e),
        }
    }
}

/// Register the `agent` tool backed by `factory`.
pub fn register_agent_tool(registry: &mut ToolRegistry, factory: Arc<dyn SessionFactory>) {
    registry.insert(Box::new(AgentTool { factory }));
}
