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
mod glob;
mod guardrails;
mod header_override;
mod identity;
mod launch;
mod lens_policy;
mod naming;
mod rules;
mod skills;
mod span;
mod target;
mod transform;

pub use federation::{Federation, FederationError, TokenClaims};
pub use gate::{Authorization, GateError, TargetFilter};
pub use guardrails::{
    AuthorizationError, CallContext, Guardrails, GuardrailsError, HeaderMutation, McpHeader,
    McpRequest, McpRequestResult, McpResponse, McpResponseResult, Outcome, Pass,
    authorization_error, request_result, response_result, to_proto_value,
};
pub use header_override::HeaderOverride;
pub use identity::{Identity, IdentityError};
pub use lens_policy::{LensPolicies, LensPolicyError, policy_digest};
pub use naming::{Resolution, ToolNamer};
pub use rules::{Call, RuleError, RuleSet, Subject};
pub use skills::{MAX_SKILL_FILE_BYTES, SKILL_SCHEME, SkillCatalog, SkillEntry, SkillError};
pub use target::{Override, SESSION_IDLE, SessionKey, Target, TargetError};
