//! Reverse-trace and domain-maturity model for `rusty_meshed`.
//!
//! Leadership asks for an *outcome* ("a dashboard showing acquisition
//! status across my programs") and perceives it as one deliverable. In
//! reality that outcome depends on data from many *domains*, each held in
//! concrete *sources* (a system, a file, a person, a process), and many of
//! those domains have not digitally transformed. This crate makes that
//! hidden plumbing explicit:
//!
//! - [`Maturity`] is the five-level ladder every domain sits on
//!   (`Tribal` .. `Integrated`).
//! - A [`Scenario`] is the full inventory: domains, their sources, and the
//!   outcomes leadership wants, each outcome declaring what it
//!   [`Requirement`]s from which domain and at what minimum maturity.
//! - [`trace`] walks one outcome back through its domains and sources and
//!   returns a [`TraceReport`]: an achievable [`Fidelity`] (`Full` /
//!   `Partial(fraction)` / `NotAchievable`), a worst-first list of
//!   [`Bottleneck`]s (the "what to fund first" list), and the classified
//!   [`TraceEdge`]s a renderer needs to draw the trace.
//!
//! Everything here is a pure function of its inputs: no I/O, no clock, no
//! randomness, so the whole thing is unit-testable with fixture scenarios.
//! Scenarios ship as TOML files ([`Scenario::from_toml`]); the same model
//! round-trips through JSON ([`Scenario::to_json`] / [`Scenario::from_json`],
//! [`TraceReport::to_json`]) so a renderer -- the `data-mesh-monitor`
//! dashboard's reverse-trace view, or a later SSE feed -- consumes exactly
//! the shape this crate produces. [`TraceReport::to_markdown`] renders the
//! same report as a "gap summary" for briefing decks.
//!
//! One representative scenario ships with the crate,
//! [`builtin::acquisition_status`] -- ~10 PAE Fires-flavoured domains at
//! mixed maturity and four leadership outcomes that stress different
//! domains. Its maturity levels are illustrative placeholders, not an
//! assessment (the spec's open question #3); replace them before showing
//! it to leadership.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod json;
mod markdown;
mod model;
mod scenario;
mod trace;

pub mod builtin;

pub use model::{
    Bottleneck, Criticality, Domain, DomainId, EdgeState, Fidelity, Maturity, ModelError, NodeRef,
    Outcome, OutcomeId, Rating, Requirement, Scenario, Source, SourceId, SourceKind, TraceEdge,
    TraceReport,
};
pub use scenario::ScenarioError;
pub use trace::{trace, trace_all, TraceError};
