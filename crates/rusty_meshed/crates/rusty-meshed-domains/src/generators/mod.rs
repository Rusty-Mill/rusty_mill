//! Synthetic scenario generation for manpower domain events -- the
//! Rust port of `meshed.domains.generators` (DOM-026..047):
//! [`scenario::ScenarioBuilder`] (a pure, in-memory causal-event
//! builder) plus the shared topic/event-map constants the
//! `run_continuous`/`run_scenario` demo binaries (`src/bin/`) build
//! on.

pub mod scenario;
pub mod topics;

pub use scenario::{ScenarioBuilder, ScenarioError, ScenarioEvent};
