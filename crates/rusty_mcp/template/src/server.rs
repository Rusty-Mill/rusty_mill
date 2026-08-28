//! The server handler and its tools.

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ErrorData, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Arguments for the `greet` tool.
///
/// Doc comments become the JSON Schema descriptions the model reads, so they
/// are worth writing for the model rather than for a maintainer.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GreetArgs {
    /// Who to greet.
    pub name: String,
    /// Greeting to use. Defaults to "Hello".
    pub greeting: Option<String>,
}

/// {{description}}
#[derive(Clone)]
pub struct Server {
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl Server {
    /// Build a handler.
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// A tool that cannot fail: return the value and the macro wraps it.
    #[tool(description = "Greet someone by name.")]
    pub async fn greet(
        &self,
        Parameters(GreetArgs { name, greeting }): Parameters<GreetArgs>,
    ) -> String {
        let greeting = greeting.unwrap_or_else(|| "Hello".to_string());
        format!("{greeting}, {name}!")
    }

    /// A tool that can fail. Return `ErrorData` for a *protocol* error — a bad
    /// argument, a missing resource. A failure the model should see and reason
    /// about belongs in the successful result instead, so it can try again.
    #[tool(description = "Divide two integers.")]
    pub async fn divide(
        &self,
        Parameters(DivideArgs { a, b }): Parameters<DivideArgs>,
    ) -> Result<String, ErrorData> {
        if b == 0 {
            return Err(ErrorData::invalid_params("cannot divide by zero", None));
        }
        Ok((a / b).to_string())
    }
}

/// Arguments for the `divide` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DivideArgs {
    /// Numerator.
    pub a: i64,
    /// Denominator. Must not be zero.
    pub b: i64,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        // `rusty_mcp::server_info` pins the advertised revision to 2026-07-28.
        // `ServerInfo::new` alone would still advertise 2025-11-25, because
        // `rmcp`'s `ProtocolVersion::LATEST` has not moved yet — and an older
        // revision means no cache hints on list results.
        rusty_mcp::server_info(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_instructions(
            "Describe here what this server is for and when to prefer one \
             tool over another. Clients show this to the model, and it is the \
             cheapest place to prevent a whole class of wrong tool choices.",
        )
    }
}
