//! The reference fixtures that survived the spike history: the `Employee`
//! domain (`employee_impl`, served by `crate::server::employee` and used by
//! the protocol, SQL and schema-driven-client tests) and the `Order`
//! dataset builder (`order_bench_support`, used by
//! `tests/generic_production_integration.rs` and
//! `benches/generic_production.rs`).
//!
//! The spikes themselves (the naive and `Dog` generic stores, the MVCC
//! spike, the rule-derivation and requirements-traceability prototypes)
//! were removed by ADR-0138; `RESULTS.md` keeps their measurements and the
//! commit ADR-0138 names keeps their code. Gated behind `research`.

pub mod employee_impl;
pub mod order_bench_support;
