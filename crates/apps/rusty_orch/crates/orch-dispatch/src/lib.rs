//! Orchestrator dispatcher: the application layer over `orch-core`.
//!
//! Routes each ready task card to an agent by role, runs it through the
//! [`AgentRunner`] port, writes what the agent produced to the [`Board`],
//! and moves the card through the [`Plan`] lifecycle. Everything is
//! synchronous and in memory; adapters that touch processes, clocks, or the
//! network live in their own crates and implement [`AgentRunner`].
//!
//! [`Board`]: orch_core::board::Board
//! [`Plan`]: orch_core::task::Plan

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod agent;
mod dispatch;
#[cfg(feature = "fake")]
pub mod fake;
mod routing;

pub use agent::{AgentError, AgentRunner, Output};
pub use dispatch::{Ceiling, DispatchError, Dispatcher, Outcome};
pub use routing::{Routing, RoutingConfig, RoutingError};
