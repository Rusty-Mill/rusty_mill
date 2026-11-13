//! Ai Env Manager core: the entry model and the link planner.
//!
//! Pure by design (pack D4): no I/O, no dependencies. An adapter observes the
//! disk and hands the planner an [`Observed`]; the planner returns a [`Plan`]
//! that is only ever applied with explicit consent (D6). The one safety rule
//! lives here: a path that does not carry our marker is never replaced or
//! removed ([`Action::Refuse`]).

mod model;
mod plan;

pub use model::{Entry, EntryId, Error, Kind, LinkMethod, Os, Tool, ToolId};
pub use plan::{plan_disable, plan_enable, Action, Observed, Plan};
