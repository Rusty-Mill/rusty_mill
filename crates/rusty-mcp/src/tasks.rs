//! The MCP Tasks extension (`io.modelcontextprotocol/tasks`, SEP-2663).
//!
//! A tool that takes minutes should not hold a request open for minutes. With
//! this extension the server answers `tools/call` with a **task handle**
//! instead of a result, and the client polls `tasks/get` until the task
//! settles.
//!
//! In 2026-07-28 this moved out of the core protocol into an opt-in extension,
//! and the blocking `tasks/result` was replaced by polling.
//!
//! # The rule that bites
//!
//! A task handle may only go to a client that **declared the extension**.
//! Returning one to a client that did not is rejected by dispatch with
//! `-32021` (`MissingRequiredClientCapability`), which surfaces as a confusing
//! failure rather than graceful degradation.
//!
//! So every task-capable tool has to work both ways. [`TaskSupport::run`]
//! decides per call and runs the same body either way, which is what keeps the
//! two paths from drifting — the usual bug being a fix applied to one and not
//! the other. Write the body against [`TaskCtx`], which degrades to no-ops
//! off-task:
//!
//! ```no_run
//! # use rmcp::{model::{CallToolResponse, CallToolResult, ContentBlock, ErrorData},
//! #            service::{RequestContext, RoleServer}, task_manager::TaskExit};
//! # use rusty_mcp::tasks::{TaskCtx, TaskSupport};
//! # struct S { tasks: TaskSupport }
//! # impl S {
//! // A normal `#[tool]` fn: taking `RequestContext` is what gives it the
//! // per-request client capabilities, and returning `CallToolResponse` is
//! // what lets it hand back a task handle.
//! async fn slow_tool(
//!     &self,
//!     ctx: RequestContext<RoleServer>,
//! ) -> Result<CallToolResponse, ErrorData> {
//!     self.tasks.run(&ctx, "slow_tool", |task: TaskCtx| async move {
//!         task.set_status_message("working");
//!         tokio::select! {
//!             _ = task.cancelled() => Err(TaskExit::Cancelled),
//!             _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
//!                 Ok(CallToolResult::success(vec![ContentBlock::text("done")]))
//!             }
//!         }
//!     })
//!     .await
//! }
//! # }
//! ```
//!
//! # Wiring
//!
//! Add a [`TaskSupport`] to your server, advertise the capability, and forward
//! the three task methods with [`crate::forward_task_methods`]:
//!
//! ```no_run
//! use rmcp::{ServerHandler, model::{ServerCapabilities, ServerInfo}};
//! use rusty_mcp::tasks::TaskSupport;
//!
//! #[derive(Clone)]
//! struct MyServer {
//!     tasks: TaskSupport,
//! }
//!
//! impl ServerHandler for MyServer {
//!     fn get_info(&self) -> ServerInfo {
//!         rusty_mcp::server_info(
//!             "my-server",
//!             "0.1.0",
//!             // Without `enable_tasks` the client never opts in, so every
//!             // call runs inline no matter what `accepts` would say.
//!             ServerCapabilities::builder().enable_tools().enable_tasks().build(),
//!         )
//!     }
//!
//!     rusty_mcp::forward_task_methods!(tasks);
//! }
//! ```

use std::{
    collections::BTreeSet,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use rmcp::{
    ErrorData,
    model::{
        CallToolResponse, CallToolResult, CancelTaskParams, CreateTaskResult, GetTaskParams,
        GetTaskResult, UpdateTaskParams,
    },
    service::{RequestContext, RoleServer},
    task_manager::{TaskContext, TaskExit, TaskManager, TaskOptions},
};

/// How often [`TaskSupport::drain`] rechecks for outstanding tasks.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Task counters, or nothing at all without the `otel` feature.
///
/// A newtype rather than a bare `Option` so the call sites below read the same
/// either way: with the feature off this is a zero-sized struct whose methods
/// compile to nothing, and `tasks.rs` needs no `cfg` noise around each call.
#[derive(Clone, Default)]
struct TaskMetrics {
    #[cfg(feature = "otel")]
    instruments: Option<Arc<crate::otel::metrics::Instruments>>,
}

impl std::fmt::Debug for TaskMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(feature = "otel")]
        let enabled = self.instruments.is_some();
        #[cfg(not(feature = "otel"))]
        let enabled = false;
        write!(f, "{enabled}")
    }
}

impl TaskMetrics {
    fn started(&self) {
        #[cfg(feature = "otel")]
        if let Some(instruments) = &self.instruments {
            instruments.task_started();
        }
    }

    fn settled(&self, _result: &Result<CallToolResult, TaskExit>) {
        #[cfg(feature = "otel")]
        if let Some(instruments) = &self.instruments {
            use crate::otel::metrics::TaskOutcome;

            instruments.task_finished(match _result {
                Ok(_) => TaskOutcome::Completed,
                Err(TaskExit::Cancelled) => TaskOutcome::Cancelled,
                Err(TaskExit::Error(_)) => TaskOutcome::Failed,
            });
        }
    }

    fn abandoned(&self, _count: usize) {
        #[cfg(feature = "otel")]
        if let Some(instruments) = &self.instruments {
            instruments.tasks_abandoned(_count);
        }
    }
}

/// Which tools run as tasks.
#[derive(Debug, Clone, Default)]
pub enum TaskPolicy {
    /// Every tool, whenever the client supports the extension.
    #[default]
    AllTools,
    /// Only the named tools. Anything else runs inline.
    ///
    /// Usually what you want: most tools return promptly, and handing back a
    /// handle for those just costs the client an extra round trip.
    Named(BTreeSet<String>),
}

impl TaskPolicy {
    /// Only these tools become tasks.
    pub fn named(tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Named(tools.into_iter().map(Into::into).collect())
    }

    /// Whether `tool` is covered.
    pub fn covers(&self, tool: &str) -> bool {
        match self {
            Self::AllTools => true,
            Self::Named(names) => names.contains(tool),
        }
    }
}

/// Task lifecycle management for a server: the manager, the policy, and the
/// capability negotiation.
///
/// Cheap to clone — the underlying [`TaskManager`] is shared, which matters
/// because Streamable HTTP builds a fresh handler per request while tasks must
/// outlive the call that created them. Construct one and clone it into each
/// handler, exactly as with any other shared state.
#[derive(Clone)]
pub struct TaskSupport {
    manager: TaskManager,
    policy: Arc<TaskPolicy>,
    poll_interval_ms: u64,
    ttl_ms: Option<u64>,
    metrics: TaskMetrics,
}

impl std::fmt::Debug for TaskSupport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskSupport")
            .field("policy", &self.policy)
            .field("poll_interval_ms", &self.poll_interval_ms)
            .field("ttl_ms", &self.ttl_ms)
            .field("running", &self.manager.running_task_count())
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl Default for TaskSupport {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskSupport {
    /// Task support covering every tool.
    pub fn new() -> Self {
        Self {
            manager: TaskManager::new(),
            policy: Arc::new(TaskPolicy::AllTools),
            // A second is a reasonable floor: fast enough that a short task
            // does not feel stalled, slow enough not to hammer the server.
            poll_interval_ms: 1_000,
            ttl_ms: Some(60 * 60 * 1_000),
            metrics: TaskMetrics::default(),
        }
    }

    /// Count tasks started, settled and abandoned.
    ///
    /// Take the instruments from [`crate::otel::OtelGuard::instruments`]. This
    /// is where the tasks extension differs from ordinary requests: the work
    /// outlives the call that created it, so an HTTP-level metrics layer sees
    /// only the request that handed out the handle, never how the task ended.
    #[cfg(feature = "otel")]
    pub fn with_metrics(mut self, instruments: Arc<crate::otel::metrics::Instruments>) -> Self {
        self.metrics.instruments = Some(instruments);
        self
    }

    /// Task support covering only the tools `policy` names.
    pub fn with_policy(policy: TaskPolicy) -> Self {
        Self {
            policy: Arc::new(policy),
            ..Self::new()
        }
    }

    /// Suggest how often clients should poll, in milliseconds.
    pub fn with_poll_interval_ms(mut self, poll_interval_ms: u64) -> Self {
        self.poll_interval_ms = poll_interval_ms;
        self
    }

    /// How long a settled task stays retrievable. `None` means unlimited.
    ///
    /// The manager evicts expired entries, so this bounds memory; too short and
    /// a client that polls slowly loses the result it was waiting for.
    pub fn with_ttl_ms(mut self, ttl_ms: impl Into<Option<u64>>) -> Self {
        self.ttl_ms = ttl_ms.into();
        self
    }

    /// The underlying manager, for cases this wrapper does not cover.
    pub fn manager(&self) -> &TaskManager {
        &self.manager
    }

    /// Whether this call should become a task.
    ///
    /// True only when the policy covers `tool` **and** the client declared the
    /// extension for this request. The capability half is not optional: a
    /// handle sent to a client that did not opt in is rejected by dispatch.
    pub fn accepts(&self, context: &RequestContext<RoleServer>, tool: &str) -> bool {
        self.policy.covers(tool) && client_supports_tasks(context)
    }

    /// Spawn `operation` as a task and return the handle to hand back.
    ///
    /// The [`TaskCtx`] lets the operation post status messages and observe
    /// cooperative cancellation. Return [`TaskExit::Cancelled`] when you notice
    /// a cancel request, so the task settles as `cancelled` rather than
    /// finishing work nobody wants.
    pub fn spawn<F, Fut>(&self, operation: F) -> CreateTaskResult
    where
        F: FnOnce(TaskCtx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<CallToolResult, TaskExit>> + Send + 'static,
    {
        let options = TaskOptions::new()
            .with_poll_interval_ms(self.poll_interval_ms)
            .with_ttl_ms(self.ttl_ms);

        let metrics = self.metrics.clone();
        metrics.started();

        let task = self.manager.spawn(options, move |ctx| {
            let running = operation(TaskCtx::spawned(ctx));
            Box::pin(async move {
                // Recorded here rather than at the call site: a task settles
                // long after `tools/call` returned, so this is the only place
                // that knows how it ended.
                let result = running.await;
                metrics.settled(&result);
                result
            })
        });

        CreateTaskResult::new(task)
    }

    /// Run `operation` as a task when the client supports it, inline otherwise.
    ///
    /// This is the shape to reach for. The same tool has to serve both kinds of
    /// client, and writing the body once here keeps the two paths from drifting
    /// apart — the usual bug being a fix applied to one and not the other.
    /// [`TaskCtx`] degrades gracefully off-task, so the body compiles once and
    /// means the right thing either way.
    ///
    /// Inline execution maps [`TaskExit::Cancelled`] to an error, since there
    /// is no task to settle as cancelled.
    pub async fn run<F, Fut>(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &str,
        operation: F,
    ) -> Result<CallToolResponse, ErrorData>
    where
        F: FnOnce(TaskCtx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<CallToolResult, TaskExit>> + Send + 'static,
    {
        if self.accepts(context, tool) {
            return Ok(CallToolResponse::Task(self.spawn(operation)));
        }

        match operation(TaskCtx::detached()).await {
            Ok(result) => Ok(result.into()),
            Err(TaskExit::Error(err)) => Err(err),
            Err(TaskExit::Cancelled) => Err(ErrorData::internal_error(
                "the operation was cancelled".to_string(),
                None,
            )),
        }
    }

    /// Serve `tasks/get`.
    pub fn get(&self, params: GetTaskParams) -> Result<GetTaskResult, ErrorData> {
        Ok(GetTaskResult::new(self.manager.get_task(&params.task_id)?))
    }

    /// Serve `tasks/update`, delivering responses to pending input requests.
    pub fn update(&self, params: UpdateTaskParams) -> Result<(), ErrorData> {
        self.manager
            .update_task(&params.task_id, params.input_responses)
    }

    /// Serve `tasks/cancel`.
    ///
    /// Cancellation is a request, not a guarantee: the acknowledgement is empty
    /// and the task settles as `cancelled` only if its body cooperates.
    pub fn cancel(&self, params: CancelTaskParams) -> Result<(), ErrorData> {
        self.manager.cancel_task(&params.task_id)
    }

    /// Tasks currently running.
    pub fn running_count(&self) -> usize {
        self.manager.running_task_count()
    }

    /// Abort running tasks immediately.
    ///
    /// Prefer [`TaskSupport::drain`], which gives work in flight a chance to
    /// finish first.
    pub fn shutdown(&self) {
        self.manager.shutdown();
    }

    /// Let running tasks finish, then abort whatever is left.
    ///
    /// Returns how many were still running when the grace period expired — a
    /// non-zero count means clients polling those task ids will never see a
    /// result, which is worth logging.
    ///
    /// Wire this into shutdown with
    /// [`ServerConfig::with_shutdown_hook`](crate::config::ServerConfig::with_shutdown_hook);
    /// without it the process exits and in-flight tasks are dropped mid-step.
    pub async fn drain(&self, grace: Duration) -> usize {
        let deadline = Instant::now() + grace;

        while self.running_count() > 0 && Instant::now() < deadline {
            tokio::time::sleep(DRAIN_POLL_INTERVAL.min(grace)).await;
        }

        let abandoned = self.running_count();
        self.metrics.abandoned(abandoned);
        self.manager.shutdown();
        abandoned
    }
}

/// Handle passed to a tool body, whether or not it is running as a task.
///
/// A tool has to serve clients that support the extension and clients that do
/// not, so its body is written once against this. Off-task the progress and
/// cancellation calls degrade to no-ops rather than being unavailable, which is
/// what lets the same code compile and behave sensibly on both paths.
#[derive(Clone)]
pub struct TaskCtx(Option<TaskContext>);

impl std::fmt::Debug for TaskCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskCtx")
            .field("task_id", &self.task_id())
            .finish()
    }
}

impl TaskCtx {
    /// Wrap a real task context.
    fn spawned(ctx: TaskContext) -> Self {
        Self(Some(ctx))
    }

    /// A context for inline execution, with nothing to report progress to.
    ///
    /// Public so a tool body can be called directly — from a test, or from the
    /// `#[tool]`-registered fallback that runs when `call_tool` did not
    /// intercept.
    pub fn detached() -> Self {
        Self(None)
    }

    /// Whether this body is running as a task.
    pub fn is_task(&self) -> bool {
        self.0.is_some()
    }

    /// The task id, when running as a task.
    pub fn task_id(&self) -> Option<&str> {
        self.0.as_ref().map(|ctx| ctx.task_id())
    }

    /// Describe what the operation is doing. Visible to the next `tasks/get`.
    ///
    /// A no-op inline, where there is nothing to poll.
    pub fn set_status_message(&self, message: impl Into<String>) {
        if let Some(ctx) = &self.0 {
            ctx.set_status_message(message);
        }
    }

    /// Whether the client has asked to cancel. Always `false` inline.
    pub fn is_cancel_requested(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(TaskContext::is_cancel_requested)
    }

    /// Resolves when cancellation is requested.
    ///
    /// Inline this never resolves, so it stays safe as a `select!` arm: the
    /// other branch simply always wins.
    pub async fn cancelled(&self) {
        match &self.0 {
            Some(ctx) => ctx.cancelled().await,
            None => std::future::pending().await,
        }
    }

    /// The underlying context, for mid-task input requests via
    /// [`TaskContext::request_input`].
    ///
    /// `None` inline — MRTR input requests need a task to hang from.
    pub fn task(&self) -> Option<&TaskContext> {
        self.0.as_ref()
    }
}

/// Whether the client declared the tasks extension for this request.
///
/// Under the stateless 2026-07-28 lifecycle capabilities arrive in each
/// request's `_meta`, so this is answered per call rather than once at startup.
pub fn client_supports_tasks(context: &RequestContext<RoleServer>) -> bool {
    context
        .client_capabilities()
        .is_some_and(|caps| caps.supports_tasks())
}

/// Implement `tasks/get`, `tasks/update` and `tasks/cancel` by forwarding to a
/// [`TaskSupport`] field.
///
/// Expands inside an `impl ServerHandler` block. The argument is the field name:
///
/// ```ignore
/// impl ServerHandler for MyServer {
///     fn get_info(&self) -> ServerInfo { /* ... */ }
///     rusty_mcp::forward_task_methods!(tasks);
/// }
/// ```
///
/// Only the three lookup methods are generated. `call_tool` stays yours,
/// because *which* tools are long-running is a design decision, not
/// boilerplate.
#[macro_export]
macro_rules! forward_task_methods {
    ($field:ident) => {
        async fn get_task(
            &self,
            request: $crate::__private::GetTaskParams,
            _context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<$crate::__private::GetTaskResult, $crate::__private::ErrorData>
        {
            self.$field.get(request)
        }

        async fn update_task(
            &self,
            request: $crate::__private::UpdateTaskParams,
            _context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<(), $crate::__private::ErrorData> {
            self.$field.update(request)
        }

        async fn cancel_task(
            &self,
            request: $crate::__private::CancelTaskParams,
            _context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<(), $crate::__private::ErrorData> {
            self.$field.cancel(request)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_policy_covers_only_its_tools() {
        let policy = TaskPolicy::named(["slow_report", "reindex"]);

        assert!(policy.covers("slow_report"));
        assert!(policy.covers("reindex"));
        assert!(!policy.covers("add"));
    }

    #[test]
    fn default_policy_covers_everything() {
        assert!(TaskPolicy::AllTools.covers("anything"));
    }

    // `TaskManager::spawn` calls `tokio::spawn`, so this needs a runtime.
    #[tokio::test]
    async fn clones_share_one_manager() {
        // Streamable HTTP builds a handler per request; a task created by one
        // must be visible to the next, or every poll would 404.
        let support = TaskSupport::new();
        let clone = support.clone();

        let handle = support.spawn(|_ctx| async move {
            Ok(CallToolResult::success(vec![
                rmcp::model::ContentBlock::text("done"),
            ]))
        });

        assert!(
            clone
                .get(GetTaskParams::new(handle.task.task_id.clone()))
                .is_ok(),
            "a clone should see tasks created through the original"
        );
    }

    #[test]
    fn unknown_task_ids_are_an_error() {
        let support = TaskSupport::new();
        assert!(support.get(GetTaskParams::new("no-such-task")).is_err());
    }
}
