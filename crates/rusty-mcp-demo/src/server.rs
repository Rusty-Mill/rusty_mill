//! The server handler: shared state plus the composed tool router.

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::router::{prompt::PromptRouter, tool::ToolRouter},
    model::{ServerCapabilities, ServerInfo},
    prompt_handler, tool_handler,
};
use rusty_mcp::tasks::{TaskPolicy, TaskSupport};
use rusty_mcp::{mrtr::InputGate, resources::ResourceRegistry, subscriptions::ChangeBroadcaster};

use crate::tools::confirm::PendingDrop;

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
    pub(crate) changes: ChangeBroadcaster,
    pub(crate) confirmations: InputGate<PendingDrop>,
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

    /// Build a server sharing both state and task manager, with a **fresh**
    /// change broadcaster.
    ///
    /// Fine for stdio, where one handler serves the whole connection. Under
    /// Streamable HTTP use [`DemoServer::with_parts`] instead: a handler is
    /// built per request, so a broadcaster created here would leave a
    /// `subscriptions/listen` request reading a channel that the request
    /// publishing the change never writes to.
    pub fn with_state_and_tasks(state: Arc<DemoState>, tasks: TaskSupport) -> Self {
        Self::with_parts(state, tasks, ChangeBroadcaster::new())
    }

    /// Build a server sharing state, tasks and the change broadcaster.
    ///
    /// The broadcaster must be cloned in rather than constructed per handler:
    /// Streamable HTTP builds a fresh handler per request, and a new channel
    /// each time would leave `subscriptions/listen` connected to nothing.
    pub fn with_parts(
        state: Arc<DemoState>,
        tasks: TaskSupport,
        changes: ChangeBroadcaster,
    ) -> Self {
        Self {
            state,
            tasks,
            changes,
            confirmations: demo_input_gate(),
            resources: crate::resources::registry(),
            // Each module contributes its own router; `+` merges them. Adding a
            // module is one more term here and nothing else.
            tool_router: Self::calculator_tools()
                + Self::text_tools()
                + Self::slow_tools()
                + Self::notify_tools()
                + Self::confirm_tools(),
            prompt_router: Self::demo_prompts(),
        }
    }
}

/// The signing key for MRTR request state.
///
/// A fixed key is fine for a demo. A real deployment must read this from
/// configuration: it has to be identical across every instance, or a retry that
/// lands on a different one behind the load balancer fails to open the state.
fn demo_input_gate() -> InputGate<PendingDrop> {
    InputGate::new(b"rusty-mcp-demo-request-state-signing-key".to_vec())
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
                // The `list_changed` flags are what let a client subscribe to
                // each category; without them the filter intersection drops it
                // and the subscription stays silent.
                .enable_resources_list_changed()
                .enable_resources_subscribe()
                .enable_prompts_list_changed()
                .enable_tool_list_changed()
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
    rusty_mcp::forward_subscription_methods!(changes);
}
