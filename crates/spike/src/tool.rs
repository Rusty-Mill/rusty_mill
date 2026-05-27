//! The aisdk ↔ harness tool seam (the BACKLOG-flagged risky seam).
//!
//! Findings driving this design (verified against aisdk 0.5.2 source):
//!
//! 1. `#[tool]` rewrites `fn read_file(path: String) -> Tool` into a *zero-arg*
//!    `fn read_file() -> Tool`, whose `execute` is a **synchronous** closure
//!    `Fn(Value) -> Result<String, String>`. It cannot host an async body and
//!    re-stringifies errors. So we use the generated `Tool` purely as a
//!    **schema carrier** (name + JSON schema) and run async execution ourselves.
//! 2. aisdk's high-level `generate_text()` loop dispatches tools via the
//!    `pub(crate)` `handle_tool_call`, with no interception point. So policy
//!    vetting (ADR-0007/0016) lives in our [`ToolRegistry::dispatch`], and the
//!    kernel drives its own loop (see [`crate::kernel`]).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use aisdk::core::tools::Tool;
use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;
use crate::outcome::ToolOutcome;
use crate::policy::Policy;

/// Object-safe plugin seam (ADR-0024): the registry holds heterogeneous tools
/// behind one type. Execution is `async`; status comes back structurally.
#[async_trait]
pub trait ToolFn: Send + Sync {
    /// Tool name surfaced to the model (matches the `#[tool]` schema name).
    fn name(&self) -> &str;
    /// JSON schema advertised to the model.
    fn schema(&self) -> Value;
    /// Execute. Policy vetting already happened in [`ToolRegistry::dispatch`].
    async fn call(&self, args: Value) -> ToolOutcome;
}

type BoxFut = Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send>>;
type AsyncInvoke = Box<dyn Fn(Value) -> BoxFut + Send + Sync>;

/// Adapts an aisdk `#[tool]` descriptor + an async body into a [`ToolFn`].
///
/// `descriptor` is the `Tool` returned by the macro-generated function; we read
/// `name` and `input_schema` from it. `invoke` is the real async body — the
/// macro's sync `execute` closure is intentionally bypassed.
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
        Fut: Future<Output = Result<String, ToolError>> + Send + 'static,
    {
        let schema =
            serde_json::to_value(&descriptor.input_schema).unwrap_or(Value::Null);
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
            Err(e) => ToolOutcome::from(e),
        }
    }
}

/// What the kernel sees (ARCHITECTURE §5): dispatch + schema advertisement.
/// The kernel never names a concrete tool type or aisdk type.
#[async_trait]
pub trait ToolDispatch: Send + Sync {
    /// Vet via policy, then execute. A block returns a `Blocked` outcome and the
    /// tool body never runs.
    async fn dispatch(&self, name: &str, args: Value) -> ToolOutcome;
    /// `(name, json_schema)` pairs to advertise to the model.
    fn schemas(&self) -> Vec<(String, Value)>;
}

/// Holds registered tools and the policy that vets every call.
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
        // Policy vets BEFORE the tool body runs (ADR-0007). On a block we never
        // reach `ToolFn::call`.
        if let Err(e) = self.policy.before_tool(name, &args).await {
            let crate::error::PolicyError::Blocked(reason) = e;
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

// --- Built-in tools ---------------------------------------------------------
//
// The `#[tool]` macro produces the schema descriptor; the async `*_impl`
// functions are the real bodies the adapter actually runs.

mod descriptors {
    use aisdk::core::tools::Tool;
    use aisdk::macros::tool;

    #[tool(name = "read_file")]
    /// Read a UTF-8 file from the workspace. `path` is workspace-relative.
    pub fn read_file_descriptor(path: String) -> Tool {
        Ok(path)
    }

    #[tool(name = "list_directory")]
    /// List the entries of a directory in the workspace.
    pub fn list_directory_descriptor(path: String) -> Tool {
        Ok(path)
    }
}

fn arg_path(args: &Value) -> Result<String, ToolError> {
    args.get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field 'path'".into()))
}

async fn read_file_impl(args: Value) -> Result<String, ToolError> {
    let path = arg_path(&args)?;
    tokio::fs::read_to_string(&path).await.map_err(|e| ToolError::Io(e.to_string()))
}

async fn list_directory_impl(args: Value) -> Result<String, ToolError> {
    let path = arg_path(&args)?;
    let mut entries = tokio::fs::read_dir(&path)
        .await
        .map_err(|e| ToolError::Io(e.to_string()))?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| ToolError::Io(e.to_string()))? {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names.join("\n"))
}

/// Register the Phase-1 built-in filesystem tools.
pub fn register_builtins(registry: &mut ToolRegistry) {
    registry.insert(Box::new(AiSdkTool::new(
        descriptors::read_file_descriptor(),
        read_file_impl,
    )));
    registry.insert(Box::new(AiSdkTool::new(
        descriptors::list_directory_descriptor(),
        list_directory_impl,
    )));
}
