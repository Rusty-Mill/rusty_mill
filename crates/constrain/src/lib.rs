#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `constrain` — the gate: vet every tool call before dispatch (ADR-0007).
//! Owns the [`Policy`] and [`ToolDispatch`] traits (ARCHITECTURE §4).

mod policy;

pub use policy::{BashGuard, Policy, PolicyChain, PolicyError, WorkspacePolicy};

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
