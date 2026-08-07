//! MCP federation.
//!
//! An `mcp` backend does not forward bytes the way an HTTP backend does — it
//! terminates the protocol. The gateway is an MCP *server* to its clients and
//! an MCP *client* to each configured target, and [`Federation`] is the piece
//! in the middle: it unions the targets' tool catalogues under qualified names
//! and routes each call back to the target that owns it.
//!
//! The two gates in [`gate`] are enforced on `tools/call` as well as
//! `tools/list`, because a tool that is hidden from the catalogue but still
//! callable is worse than one that was never hidden.

mod federation;
mod gate;
mod naming;
mod rules;
mod target;

pub use federation::{Federation, FederationError, TokenClaims};
pub use gate::{Authorization, GateError, TargetFilter};
pub use naming::{Resolution, ToolNamer};
pub use rules::{RuleError, RuleSet, ToolCall};
pub use target::{Target, TargetError};
