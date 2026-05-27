//! The aisdk ↔ harness tool seam (spike 01).
//!
//! The `#[tool]` macro emits a *synchronous* zero-arg descriptor function
//! returning an aisdk `Tool`; we use that `Tool` purely as a **schema carrier**
//! and run the real async body in [`ToolFn::call`]. The macro's sync `execute`
//! closure is intentionally bypassed. See `docs/spike/01-aisdk-tool-seam.md`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use aisdk::core::tools::Tool;
use async_trait::async_trait;
use rk_constrain::{Policy, ToolDispatch};
use rk_observe::ToolOutcome;
use serde_json::Value;

/// Object-safe plugin seam (ADR-0024): the registry holds heterogeneous tools
/// behind one type. Execution is `async`; status comes back structurally.
#[async_trait]
pub trait ToolFn: Send + Sync {
    /// Tool name surfaced to the model.
    fn name(&self) -> &str;
    /// JSON schema advertised to the model.
    fn schema(&self) -> Value;
    /// Execute. Policy vetting already happened in [`ToolRegistry::dispatch`].
    async fn call(&self, args: Value) -> ToolOutcome;
}

type BoxFut = Pin<Box<dyn Future<Output = Result<String, crate::error::ToolError>> + Send>>;
type AsyncInvoke = Box<dyn Fn(Value) -> BoxFut + Send + Sync>;

/// Adapts an aisdk `#[tool]` descriptor + an async body into a [`ToolFn`].
pub struct AiSdkTool {
    name: String,
    schema: Value,
    invoke: AsyncInvoke,
}

impl AiSdkTool {
    /// Build from a macro-generated descriptor and an async invoke closure.
    pub fn new<F, Fut>(descriptor: Tool, invoke: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, crate::error::ToolError>> + Send + 'static,
    {
        let schema = serde_json::to_value(&descriptor.input_schema).unwrap_or(Value::Null);
        Self {
            name: descriptor.name,
            schema,
            invoke: Box::new(move |args| Box::pin(invoke(args))),
        }
    }
}

#[async_trait]
impl ToolFn for AiSdkTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        match (self.invoke)(args).await {
            Ok(s) => ToolOutcome::ok(s),
            Err(e) => crate::error::outcome_from_error(e),
        }
    }
}

/// Holds registered tools and the policy that vets every call. Implements the
/// `constrain::ToolDispatch` seam the kernel consumes.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn ToolFn>>,
    policy: Arc<dyn Policy>,
}

impl ToolRegistry {
    /// New empty registry guarded by `policy`.
    pub fn new(policy: Arc<dyn Policy>) -> Self {
        Self { tools: HashMap::new(), policy }
    }

    /// Register a tool under its own name.
    pub fn insert(&mut self, tool: Box<dyn ToolFn>) {
        self.tools.insert(tool.name().to_string(), tool);
    }
}

#[async_trait]
impl ToolDispatch for ToolRegistry {
    async fn dispatch(&self, name: &str, args: Value) -> ToolOutcome {
        // Policy vets BEFORE the tool body runs (ADR-0007); on a block we never
        // reach `ToolFn::call`.
        if let Err(rk_constrain::PolicyError::Blocked(reason)) =
            self.policy.before_tool(name, &args).await
        {
            return ToolOutcome::blocked(reason);
        }
        match self.tools.get(name) {
            Some(tool) => tool.call(args).await,
            None => ToolOutcome::error(format!("unknown tool '{name}'")),
        }
    }

    fn schemas(&self) -> Vec<(String, Value)> {
        let mut out: Vec<_> =
            self.tools.values().map(|t| (t.name().to_string(), t.schema())).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}
