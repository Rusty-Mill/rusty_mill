//! A deliberately slow tool, to exercise the tasks extension.
//!
//! The body is written once against [`TaskCtx`] and runs either way:
//! as a task for clients that declared `io.modelcontextprotocol/tasks`,
//! inline for everyone else. `TaskSupport::run` makes that choice per call.

use std::time::Duration;

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResponse, CallToolResult, ContentBlock, ErrorData},
    service::{RequestContext, RoleServer},
    task_manager::TaskExit,
    tool, tool_router,
};
use rusty_mcp::tasks::TaskCtx;
use schemars::JsonSchema;
use serde::Deserialize;

/// How long to pretend to work for.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountdownArgs {
    /// Number of steps to run. Each step takes about 50ms.
    pub steps: u32,
}

/// The tool name, so the task policy and the tool registration cannot drift.
pub const COUNTDOWN: &str = "countdown";

#[tool_router(router = slow_tools, vis = "pub(crate)")]
impl crate::server::DemoServer {
    /// Count down slowly, as a task where the client supports one.
    ///
    /// Taking `RequestContext` as a second parameter is what gives the tool
    /// access to the per-request client capabilities — under the stateless
    /// 2026-07-28 lifecycle those arrive in each request's `_meta`, not once at
    /// startup, so the decision is made per call.
    #[tool(
        name = "countdown",
        description = "Count down in steps, slowly. Returns a task handle to clients that support the tasks extension."
    )]
    pub async fn countdown(
        &self,
        Parameters(args): Parameters<CountdownArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.tasks
            .run(&ctx, COUNTDOWN, move |task_ctx| {
                countdown_body(args, task_ctx)
            })
            .await
    }
}

/// The actual work, shared by both execution paths.
pub async fn countdown_body(args: CountdownArgs, ctx: TaskCtx) -> Result<CallToolResult, TaskExit> {
    // Bound the work: an unbounded `steps` from a caller would pin a worker.
    let steps = args.steps.min(200);

    for remaining in (1..=steps).rev() {
        ctx.set_status_message(format!("{remaining} steps remaining"));

        tokio::select! {
            // Off-task this branch never fires, so the sleep always wins.
            _ = ctx.cancelled() => return Err(TaskExit::Cancelled),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }

    Ok(CallToolResult::success(vec![ContentBlock::text(format!(
        "counted down {steps} steps"
    ))]))
}
