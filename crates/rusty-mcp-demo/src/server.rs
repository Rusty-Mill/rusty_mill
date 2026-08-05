//! The server handler: shared state plus the composed tool router.

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::router::{prompt::PromptRouter, tool::ToolRouter},
    model::{ServerCapabilities, ServerInfo},
    prompt_handler, tool_handler,
};
use rusty_mcp::resources::ResourceRegistry;
use rusty_mcp::tasks::{TaskPolicy, TaskSupport};

use crate::tools::slow::COUNTDOWN;

/// State shared across every tool call.
///
/// Streamable HTTP builds a handler per request, so anything expensive belongs
/// here behind the `Arc` rather than in [`DemoServer`] itself.
#[derive(Debug, Default)]
pub struct DemoState {
    /// Bumped by tools that want to show state surviving across calls.
    pub calls: std::sync::atomic::AtomicU64,
}

/// Demo server exposing a few small, dependency-free tools.
#[derive(Clone)]
pub struct DemoServer {
    pub(crate) state: Arc<DemoState>,
    pub(crate) tasks: TaskSupport,
    pub(crate) resources: ResourceRegistry,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

impl DemoServer {
    /// Build a server with a fresh state handle.
    pub fn new() -> Self {
        Self::with_state(Arc::new(DemoState::default()))
    }

    /// Build a server sharing an existing state handle.
    ///
    /// Under Streamable HTTP the factory closure should capture one `Arc` and
    /// call this, so every request-scoped handler sees the same state.
    pub fn with_state(state: Arc<DemoState>) -> Self {
        Self::with_state_and_tasks(state, default_task_support())
    }

    /// Build a server sharing both state and task manager.
    ///
    /// Tasks outlive the call that created them, and Streamable HTTP builds a
    /// fresh handler per request — so the factory must clone one `TaskSupport`
    /// in rather than construct a new one, or every poll would miss.
    pub fn with_state_and_tasks(state: Arc<DemoState>, tasks: TaskSupport) -> Self {
        Self {
            state,
            tasks,
            resources: crate::resources::registry(),
            // Each module contributes its own router; `+` merges them. Adding a
            // module is one more term here and nothing else.
            tool_router: Self::calculator_tools() + Self::text_tools() + Self::slow_tools(),
            prompt_router: Self::demo_prompts(),
        }
    }
}

/// How long this process has been running.
///
/// Backs the `status://uptime` resource, which exists to show a resource whose
/// content is produced per read rather than fixed at startup.
pub fn process_uptime() -> std::time::Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// Only `countdown` is slow enough to be worth a task handle; the arithmetic
/// and text tools return immediately, and handing back a handle for those would
/// just cost the client an extra round trip.
pub fn default_task_support() -> TaskSupport {
    TaskSupport::with_policy(TaskPolicy::named([COUNTDOWN])).with_poll_interval_ms(50)
}

impl Default for DemoServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for DemoServer {
    fn get_info(&self) -> ServerInfo {
        // `rusty_mcp::server_info` pins the advertised revision to 2026-07-28;
        // `ServerInfo::new` alone would still advertise 2025-11-25.
        rusty_mcp::server_info(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .enable_tasks()
                .build(),
        )
        .with_instructions(
            "Small arithmetic and text utilities, used to demonstrate the \
                 rusty-mcp scaffold. Prefer `divide` over `add` when you need \
             the remainder as well. Resources expose configuration, uptime and \
             table schemas; prompts cover summarizing text and explaining \
             errors.",
        )
    }

    rusty_mcp::forward_task_methods!(tasks);
    rusty_mcp::forward_resource_methods!(resources);
}
