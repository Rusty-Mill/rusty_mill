//! [`FunctionTool`] — wraps a Rust closure as a [`Tool`].

use adk_core::{Args, FunctionDeclaration, Result, Schema};
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;

use crate::context::ToolContext;
use crate::tool::{ConfirmationPolicy, Tool};

/// The future a tool closure returns, borrowing its [`ToolContext`].
pub type ToolFuture<'a> = BoxFuture<'a, Result<Value>>;

/// The callable a [`FunctionTool`] wraps, before being shared.
pub trait ToolCallable: for<'a> Fn(Args, &'a ToolContext) -> ToolFuture<'a> + Send + Sync {}

impl<T> ToolCallable for T where T: for<'a> Fn(Args, &'a ToolContext) -> ToolFuture<'a> + Send + Sync
{}

/// The closure shape a [`FunctionTool`] wraps.
pub type ToolFn = Arc<dyn ToolCallable>;

/// A [`Tool`] built from a closure, with a hand-written declaration.
///
/// The `#[adk_tool]` macro in `adk-macros` builds one of these from a plain
/// `async fn`, deriving the schema from the signature and the description from
/// the doc comment. Construct one directly when the schema is dynamic or the
/// function is not known at compile time.
///
/// # Example
///
/// ```
/// use adk_core::Schema;
/// use adk_tools::FunctionTool;
/// use serde_json::json;
///
/// let tool = FunctionTool::new(
///     "get_weather",
///     "Retrieves the current weather for a city.",
///     Schema::object().property("city", Schema::string().describe("The city name.")),
///     |args, _ctx| {
///         Box::pin(async move {
///             let city = args.get("city").and_then(|v| v.as_str()).unwrap_or("unknown");
///             Ok(json!({ "status": "success", "report": format!("Sunny in {city}.") }))
///         })
///     },
/// );
/// ```
pub struct FunctionTool {
    name: String,
    description: String,
    parameters: Option<Schema>,
    response: Option<Schema>,
    long_running: bool,
    confirmation: ConfirmationPolicy,
    func: ToolFn,
}

impl FunctionTool {
    /// Builds a tool from a name, description, parameter schema, and closure.
    pub fn new<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Schema,
        func: F,
    ) -> Self
    where
        F: for<'a> Fn(Args, &'a ToolContext) -> BoxFuture<'a, Result<Value>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: Some(parameters),
            response: None,
            long_running: false,
            confirmation: ConfirmationPolicy::Never,
            func: Arc::new(func),
        }
    }

    /// Builds a tool that takes no arguments.
    pub fn nullary<F>(name: impl Into<String>, description: impl Into<String>, func: F) -> Self
    where
        F: for<'a> Fn(Args, &'a ToolContext) -> BoxFuture<'a, Result<Value>>
            + Send
            + Sync
            + 'static,
    {
        let mut tool = Self::new(name, description, Schema::object(), func);
        tool.parameters = None;
        tool
    }

    /// Declares the tool's return schema, for providers that use it.
    pub fn with_response_schema(mut self, response: Schema) -> Self {
        self.response = Some(response);
        self
    }

    /// Marks the tool as running in the background.
    pub fn long_running(mut self) -> Self {
        self.long_running = true;
        self
    }

    /// Requires user approval before every call.
    pub fn require_confirmation(mut self, hint: impl Into<String>) -> Self {
        self.confirmation = ConfirmationPolicy::Always(hint.into());
        self
    }

    /// Requires user approval only when `predicate` returns a prompt.
    pub fn require_confirmation_when<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&Args) -> Option<String> + Send + Sync + 'static,
    {
        self.confirmation = ConfirmationPolicy::Conditional(Box::new(predicate));
        self
    }

    /// Wraps this tool in an [`Arc`] for registration with an agent.
    pub fn shared(self) -> Arc<dyn Tool> {
        Arc::new(self)
    }
}

#[async_trait]
impl Tool for FunctionTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn declaration(&self) -> Option<FunctionDeclaration> {
        let mut decl = FunctionDeclaration::new(&self.name, &self.description);
        decl.parameters = self.parameters.clone();
        decl.response = self.response.clone();
        Some(decl)
    }

    fn is_long_running(&self) -> bool {
        self.long_running
    }

    fn confirmation_hint(&self, args: &Args) -> Option<String> {
        self.confirmation.hint_for(args)
    }

    async fn run(&self, args: Args, ctx: &ToolContext) -> Result<Value> {
        (self.func)(args, ctx).await
    }
}

impl std::fmt::Debug for FunctionTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionTool")
            .field("name", &self.name)
            .field("long_running", &self.long_running)
            .field("confirmation", &self.confirmation)
            .finish_non_exhaustive()
    }
}
