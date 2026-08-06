//! An AI-native gateway that speaks agentgateway's configuration.
//!
//! Point it at a `config.yaml` and it binds every port the file names, routes
//! by Gateway API precedence, and terminates MCP for `mcp` backends —
//! federating their targets into one endpoint.
//!
//! The binary is a thin wrapper over [`gateway::Gateway`] and [`serve`], so
//! integration tests can drive the same data plane the binary runs.

pub mod gateway;
pub mod serve;
pub mod telemetry;

pub use gateway::Gateway;
pub use telemetry::Telemetry;
