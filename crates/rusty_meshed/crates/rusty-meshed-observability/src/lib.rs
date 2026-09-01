//! Lineage tracking, metrics collection, SLO monitoring, and the CI
//! contract-compatibility gate -- the Rust port of
//! `meshed.observability` (`LineageTracker`, `MetricsCollector`,
//! `SLOMonitor`/`SLOViolationPublisher`, `contract_gate`).
//!
//! See `../../capability-manifest.md` (rows GOV-021..049) for the
//! capabilities this crate covers; most are still open, see the
//! crate's module list below for what's implemented so far.

mod contract_gate;
mod lineage;
mod metrics;

pub use contract_gate::{
    assert_schema_compatible, contract_subject_name, register_consumer_contract, ContractGateError,
};
pub use lineage::{LineageRecord, LineageTracker, TopologyDependency};
pub use metrics::{
    ensure_schema as ensure_metrics_schema, get_violation_count, record_violation,
    MetricsCollector, MetricsError, ProductMetrics,
};
