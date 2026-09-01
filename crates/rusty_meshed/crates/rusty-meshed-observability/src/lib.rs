//! Lineage tracking, metrics collection, SLO monitoring, and the CI
//! contract-compatibility gate -- the Rust port of
//! `meshed.observability` (`LineageTracker`, `MetricsCollector`,
//! `SLOMonitor`/`SLOViolationPublisher`, `contract_gate`).
//!
//! See `../../capability-manifest.md` (rows GOV-021..049) for the
//! capabilities this crate covers; most are still open, see the
//! crate's module list below for what's implemented so far.

mod lineage;

pub use lineage::{LineageRecord, LineageTracker, TopologyDependency};
