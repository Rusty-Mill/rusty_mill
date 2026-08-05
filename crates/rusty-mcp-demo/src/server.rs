//! The server handler: shared state plus the composed tool router.

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{ServerCapabilities, ServerInfo},
    tool_handler,
};

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
    tool_router: ToolRouter<Self>,
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
        Self {
            state,
            // Each tool module contributes its own router; `+` merges them.
            // Adding a module is one more term here and nothing else.
            tool_router: Self::calculator_tools() + Self::text_tools(),
        }
    }
}

impl Default for DemoServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DemoServer {
    fn get_info(&self) -> ServerInfo {
        // `rusty_mcp::server_info` pins the advertised revision to 2026-07-28;
        // `ServerInfo::new` alone would still advertise 2025-11-25.
        rusty_mcp::server_info(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_instructions(
            "Small arithmetic and text utilities, used to demonstrate the \
                 rusty-mcp scaffold. Prefer `divide` over `add` when you need \
             the remainder as well.",
        )
    }
}
