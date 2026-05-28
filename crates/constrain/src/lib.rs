#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `constrain` — the gate: vet every tool call before dispatch (ADR-0007).
//! Owns the [`Policy`] and [`ToolDispatch`] traits (ARCHITECTURE §4).

mod approval;
mod plan;
mod policy;
mod security;

pub use approval::{ApprovalGate, ApprovalRequest, ApprovalResponse, ApprovalTrigger};
pub use plan::{PlanController, PlanDecision};
pub use policy::{
    within_workspace, AcpPolicy, BashGuard, ModePolicy, PermissionMode, Policy, PolicyChain,
    PolicyError, WorkspacePolicy,
};
pub use security::{
    default_checkers, CommandInjectionCheck, DestructiveCommandCheck, NetworkExfilCheck,
    PathTraversalCheck, PrivilegeEscalationCheck, SecurityCheck, SecurityLog,
};

use async_trait::async_trait;
use rk_observe::ToolOutcome;
use serde_json::Value;

/// What the kernel sees (ARCHITECTURE §5): a single object-safe seam that vets
/// and dispatches. The kernel never names a concrete tool or registry type.
///
/// Implemented by `feed::ToolRegistry`. Dispatch is policy-vetted: a blocked
/// call returns a `Blocked` [`ToolOutcome`] and the tool body never runs.
#[async_trait]
pub trait ToolDispatch: Send + Sync {
    /// Vet via policy, then execute.
    async fn dispatch(&self, name: &str, args: Value) -> ToolOutcome;
    /// `(name, json_schema)` pairs to advertise to the model.
    fn schemas(&self) -> Vec<(String, Value)>;
}
